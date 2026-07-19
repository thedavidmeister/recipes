#!/usr/bin/env bash
#
# Enrichment worker cron (#59): drain the recipes enrichment queue.
#
# Runs Claude Code headless against the recipes-enrich plugin MCP tools
# (enrich_pull / enrich_push) and loops until the queue is empty. The model reading
# runs under this machine logged-in Claude Code (the Max plan), so marginal
# inference cost is ~$0. The worker never touches the database: it reaches the corpus
# only through the app INGEST_API_KEY-gated endpoints, via the MCP server the plugin
# ships (recipe-backend mcp). Nothing secret is committed: config comes from an env
# file (see scripts/enrich.env.example).
#
# Schedule with `crontab -e`, e.g. every 4 hours:
#   0 */4 * * * /home/gildlab/code/recipes/scripts/enrich-cron.sh >> "$HOME/recipes-enrich.log" 2>&1
#
# Requires on PATH: claude, nix (cron has a minimal PATH; see the bootstrap below).

set -euo pipefail

REPO="${RECIPES_REPO:-/home/gildlab/code/recipes}"
ENV_FILE="${RECIPES_ENRICH_ENV:-$REPO/.env.enrich}"
MODEL="${ENRICH_CLAUDE_MODEL:-sonnet}"   # session model that does the reading
MAX_BATCHES="${ENRICH_MAX_BATCHES:-20}"  # claude sessions per run; leftover waits for the next run
SESSION_TIMEOUT="${ENRICH_SESSION_TIMEOUT:-900}" # wall-clock cap per session (there is no --max-turns)

log() { printf '%s enrich-cron %s\n' "$(date -u +%FT%TZ)" "$*"; }
die() { log "ERROR: $*"; exit 1; }

# --- PATH bootstrap: cron runs with a minimal environment ------------------------
# Make nix available, then rely on the login profile for claude. Adjust here if your
# install lives elsewhere.
if [ -e "$HOME/.nix-profile/etc/profile.d/nix.sh" ]; then
  # shellcheck source=/dev/null
  . "$HOME/.nix-profile/etc/profile.d/nix.sh"
fi
command -v claude >/dev/null 2>&1 || die "claude not on PATH (cron PATH is minimal; add its dir)"
command -v nix >/dev/null 2>&1 || die "nix not on PATH"

# --- config: RECIPES_API_URL, INGEST_API_KEY, ENRICH_MODEL -----------------------
[ -f "$ENV_FILE" ] || die "missing env file $ENV_FILE (copy scripts/enrich.env.example and fill it)"
set -a
# shellcheck source=/dev/null
. "$ENV_FILE"
set +a
: "${RECIPES_API_URL:?set RECIPES_API_URL in the env file}"
: "${INGEST_API_KEY:?set INGEST_API_KEY in the env file}"
: "${ENRICH_MODEL:?set ENRICH_MODEL in the env file, e.g. claude-sonnet-5}"
# Exported so the MCP server subprocess Claude Code spawns inherits them.
export RECIPES_API_URL INGEST_API_KEY ENRICH_MODEL

cd "$REPO"

# --- track main ------------------------------------------------------------------
# The cron builds recipe-backend, the plugin, and the skill from this checkout, so
# pull the latest merged code first. Only when the tree is clean AND on main, though:
# this checkout is also used interactively, and the cron must never switch a branch or
# clobber uncommitted work out from under a session mid-edit. Off-main or dirty => skip
# with a warning and build the working tree as it stands.
branch="$(git -C "$REPO" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
if [ -n "$(git -C "$REPO" status --porcelain 2>/dev/null)" ]; then
  log "WARNING: $REPO is dirty; skipping pull, building the working tree as-is"
elif [ "$branch" != "main" ]; then
  log "WARNING: $REPO is on '$branch', not main; skipping pull, building it as-is"
else
  log "updating $REPO to origin/main"
  git -C "$REPO" pull --ff-only --quiet origin main \
    || die "git pull --ff-only failed (has main diverged locally?)"
fi

# --- recipe-backend on PATH ------------------------------------------------------
# The MCP server (recipe-backend mcp) and the CLI queue-peek below both need the
# binary. Build it as a flake package (cached after the first run) and prepend its
# bin dir; this tracks the repo checkout with no manual cargo build.
log "resolving recipe-backend via nix"
backend_bin="$(nix build "$REPO#recipe-backend" --no-link --print-out-paths)" || die "nix build failed"
export PATH="$backend_bin/bin:$PATH"

PLUGIN_DIR="$REPO/plugins/recipes-enrich"
SKILL_FILE="$PLUGIN_DIR/skills/enrich/SKILL.md"
[ -f "$SKILL_FILE" ] || die "missing skill file $SKILL_FILE"

# Plugin MCP tool names are mcp__plugin_<plugin>_<server>__<tool> (verified against
# Claude Code 2.1.x with --plugin-dir). Allow-list only these two: the worker needs
# nothing else, and print mode never prompts.
ALLOWED="mcp__plugin_recipes-enrich_recipes-enrich__enrich_pull,mcp__plugin_recipes-enrich_recipes-enrich__enrich_push"

PROMPT="Drain the recipes enrichment queue now. Loop: call enrich_pull, read each \
returned recipe ingredient lines into StructuredMeasure readings, then call \
enrich_push with them; repeat until enrich_pull returns an empty array. Use only the \
enrich_pull and enrich_push tools."

# --- drain loop ------------------------------------------------------------------
# Peek the queue cheaply via the CLI (a read-only pull; it does not consume). If it
# is empty, stop. Otherwise run one bounded Claude session, which itself loops until
# the queue drains or the timeout hits; the outer loop just re-runs for any
# remainder, so a large backfill drains over several sessions instead of one
# unbounded one.
for batch in $(seq 1 "$MAX_BATCHES"); do
  # A failed peek (backend down, bad key) must NOT read as an empty queue, or the
  # cron would exit "drained" and hide the outage from monitoring; fail loudly.
  pending="$(recipe-backend enrich pull --limit 1)" \
    || die "queue peek failed; backend unreachable or misconfigured"
  if [ "$pending" = "[]" ]; then
    log "queue empty; drained after $((batch - 1)) session(s)"
    exit 0
  fi

  log "session $batch: draining (model=$MODEL)"
  # A session loops internally until the queue drains or the wall-clock cap. A
  # timeout (exit 124) is expected mid-backfill, so continue to the next batch; any
  # other non-zero exit is a real failure worth stopping on (the next cron retries).
  rc=0
  timeout "$SESSION_TIMEOUT" claude -p "$PROMPT" \
    --plugin-dir "$PLUGIN_DIR" \
    --append-system-prompt "$(< "$SKILL_FILE")" \
    --model "$MODEL" \
    --allowedTools "$ALLOWED" || rc=$?
  if [ "$rc" -eq 124 ]; then
    log "session $batch hit the ${SESSION_TIMEOUT}s cap; continuing"
  elif [ "$rc" -ne 0 ]; then
    die "claude session $batch failed (exit $rc); next cron run will retry"
  fi
done

log "hit MAX_BATCHES=$MAX_BATCHES; any remainder waits for the next run"
