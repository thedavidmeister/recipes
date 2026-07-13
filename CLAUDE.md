# recipes

Cooking **recipe aggregator** (NOT a CRUD app / CMS — it normalizes existing
public recipe sources into a searchable corpus). Client-heavy design; see the
diagram in [README.md](./README.md).

- **`crates/recipe-core`** — shared Rust: models + schema.org + TheMealDB
  normalization. Compiles **native** (backend) _and_ **wasm32** (browser).
- **`crates/recipe-wasm`** — wasm-bindgen wrapper → npm package the frontend
  imports.
- **`backend/`** — thin Rust/Axum on **Render**: a **fetch-proxy** (fetches
  external pages server-side to bypass CORS/bot walls) + **Turso write-gateway**
  (holds the write token). It does NOT parse.
- **`frontend/`** — SvelteKit SPA (`adapter-static`): parses/normalizes via
  recipe-core **WASM**, reads **Turso** directly (read-only token), calls the
  backend to fetch and to write. TanStack Query · Bits UI · Tailwind.
- **Turso** (libSQL/SQLite) is the corpus/store. Dev env + CI via **rainix**
  (`nix develop` = rainix `wasm-shell`: Rust + wasm-pack + Node).

## Working memory — READ THIS FIRST, EVERY SESSION

**`.worklog/journal.md`** (gitignored) is the durable working memory for this
project — architecture decisions, the pivots that got us here, open questions,
build status, and a running log.

- **Read it at the start of every session**, before acting.
- **Keep it updated as you work**, and journal important or compaction-fragile
  details there so they survive context compaction.
- It is distinct from global `~/.claude` memory (which is behavioural/process,
  not project-specific).

## Conventions

- Parsing/normalization lives in **`recipe-core` once** (native + wasm) — never
  duplicate it in the backend or re-implement it in JS.
- The backend is a **fetch-proxy + write-gateway ONLY**. All writes go through
  it — the Turso _write_ token never reaches the browser (the browser gets a
  read-only token). **SSRF-guard every proxied fetch**: http(s) only; block
  private/loopback/link-local/metadata IPs; timeout; size + redirect caps.
- **Turso is the store** — there is no server-side cache layer.
- Keep the WASM bundle lean — avoid heavy deps: `recipe-core` extracts JSON-LD
  with the lightweight `tl` tokenizer (a real parser, not html5ever/scraper,
  which bloat wasm and pull in `getrandom`). Not regex.
- Formatting and CI come from **rainix**. Do NOT add `prettier`,
  `prettier-plugin-*`, or a `.prettierrc` to the frontend — rainix's
  `no-consumer-prettier` pre-commit hook forbids it; the curated bundle
  (exported via `RAINIX_PRETTIER_BUNDLE_DIR` in the dev shell) is the only
  prettier in play.
