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
- **`frontend/`** — SvelteKit SPA (`adapter-static`) on a **Render static
  site**: parses/normalizes via recipe-core **WASM**, reads **Turso** directly
  (read-only token), calls the backend to fetch and to write. TanStack Query ·
  Bits UI · Tailwind. UI states are declared as **Storybook** stories.
- **Turso** (libSQL/SQLite) is the corpus/store. Dev env + CI via **rainix**
  (`nix develop` = rainix `wasm-shell`: Rust + wasm-pack + Node).

## The stack is Render + Turso — that is the whole vendor list

Do **not** reintroduce Vercel, Cloudflare (Pages/Workers/D1/R2), IPFS, or any
other host: they were considered and rejected (see README "Why these choices").
Backend = Render web service (free, spins down at 15 min idle). Frontend +
Storybook = Render **static site** (permanently free, never spins down, no
card). Render's **500 build-minutes/month are shared** across the workspace and
belong to deploys — don't design work that burns them.

## Working memory — READ THIS FIRST, EVERY SESSION

**`.worklog/journal.md`** (gitignored) is the durable working memory for this
project — architecture decisions, the pivots that got us here, open questions,
build status, and a running log.

- **Read it at the start of every session**, before acting.
- **Keep it updated as you work**, and journal important or compaction-fragile
  details there so they survive context compaction.
- It is distinct from global `~/.claude` memory (which is behavioural/process,
  not project-specific).

**Settled decisions belong on their GitHub issue, not only in the journal** — an
issue is the shared, durable record. Write the ruling down when it's made;
re-deriving a settled decision wastes the human's time. Live design notes:

- **#21 — screenshots** (Storybook capture harness; the chromium/fonts recipe;
  what's ruled out; the one open question). Read it before touching screenshots.
- **#20 — cook-decider** (realtime transport decided: WS for liveness + Turso
  for persistence).

## Conventions

- Parsing/normalization lives in **`recipe-core` once** (native + wasm) — never
  duplicate it in the backend or re-implement it in JS.
- The backend is a **fetch-proxy + write-gateway ONLY**. All writes go through
  it — the Turso _write_ token never reaches the browser (the browser gets a
  read-only token). **SSRF-guard every proxied fetch**: http(s) only; block
  private/loopback/link-local/metadata IPs; timeout; size + redirect caps.
- **Turso is the store** — there is no server-side cache layer.
- **Every UI state is a Storybook story**, not something you reach by driving
  the live app. `Pending`/`Error`/`Empty` are impractical to click to, and it
  gets worse as states multiply. Components take state as props (e.g.
  `SearchResults` takes `status: idle|pending|error|ready`); the page owns the
  query, the component owns rendering. Story fixtures mirror **real** source
  records — invented ids/images render as the wrong meal. Stories use
  `const meta = {…} satisfies Meta<typeof Cmp>`, never a `Meta<…>` annotation
  (an annotation breaks `StoryObj` arg inference). Storybook and screenshots are
  complementary: Storybook declares the states, screenshots pin the work at hand
  onto a PR. See #21.
- Keep the WASM bundle lean — avoid heavy deps: `recipe-core` extracts JSON-LD
  with the lightweight `tl` tokenizer (a real parser, not html5ever/scraper,
  which bloat wasm and pull in `getrandom`). Not regex.
- Formatting and CI come from **rainix**. Do NOT add `prettier`,
  `prettier-plugin-*`, or a `.prettierrc` to the frontend — rainix's
  `no-consumer-prettier` pre-commit hook forbids it; the curated bundle
  (exported via `RAINIX_PRETTIER_BUNDLE_DIR` in the dev shell) is the only
  prettier in play.
