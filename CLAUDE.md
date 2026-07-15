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

## The infra today is Render + Turso + Cloudflare R2 (screenshots only)

Backend = Render web service (free, spins down at 15 min idle). Frontend +
Storybook = Render **static site** (permanently free, never spins down).
Render's **500 build-minutes/month are shared** across the workspace and belong
to deploys — don't design work that burns them. **Cloudflare R2** holds PR
screenshots only (bucket `lehlehleh`) — genuinely $0 at this size, egress always
free. Render has **no object storage** (MinIO would need a paid instance+disk).

That list is a **fact, not a ban**. Don't claim anything else is "already in our
stack" (Vercel/Cloudflare are not) — but adding a service when something needs
one is a normal decision, not a forbidden move. Vercel/Cloudflare were ruled out
as hosts for _this Rust backend_ specifically; that says nothing about, say,
Cloudflare R2 as an object store.

**Do not invent constraints.** The real bar for a dependency is: **is it
actually free at our size** (verify the bill, not the marketing — Shuttle's free
tier ended; Fly.io removed its free allowances). Things that are **NOT**
constraints, and must not be treated as such: needing a **credit card** is fine;
**link rot** is fine; regenerable artifacts like screenshots **need no
expiry/deletion**. Constraints come from the human — don't harden an observation
into a rule and then optimise against it.

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
  (an annotation breaks `StoryObj` arg inference).

## Screenshots on a UI PR (settled — see #21)

Storybook and screenshots are **complementary**: Storybook _declares_ the
states, screenshots _pin the work at hand_ onto the PR. A UI PR wants both. Run
this by hand when a PR needs shots — it is deliberately **not** in CI.

```sh
(cd frontend && npm ci && npm run build-storybook)
nix run .#storybook-shot                       # every story -> ./screenshots
WIDTH=760 HEIGHT=90 nix run .#storybook-shot -- 'searchresults--(pending|error)'
```

Then upload to R2 and embed the **public** URL in a PR comment (`.env.example`
documents the vars; secrets live in gitignored `.env`):

```sh
rclone copy screenshots/ "R2:$R2_BUCKET/recipes/pr-<n>/" --header-upload "Content-Type: image/png"
# embed: $R2_PUBLIC_BASE/recipes/pr-<n>/<story-id>.png
```

Hard-won details — don't rediscover these:

- **GitHub has no API to attach an image to a comment**
  (`/upload/policies/assets` is web-UI-only, rejects PAT auth). Hence hosting +
  embedding by URL.
- The **public** host (`pub-*.r2.dev`) is NOT the S3 endpoint, and only exists
  once the bucket has _Public access → Allow_. Camo fetches server-side, so the
  URL must be unsigned/non-expiring. **Verify each URL returns `200 image/png`
  anonymously before posting** — a 404 embed looks like success.
- Upload with **rclone**: awscli's TLS fails against R2 here, and system curl
  7.81's `--aws-sigv4` omits R2's required `x-amz-content-sha256`.
- Scope the R2 token to **Object Read & Write on one bucket** — never a global
  Cloudflare key. It intentionally can't list buckets.
- `screenshots/` is gitignored. Never commit PNGs; never use an orphan branch.
- The capture traps live in `flake.nix` (`storybook-shot`): `pkgs.chromium`, not
  `ungoogled-chromium` (crashes headless); and a fonts.conf with generic
  aliases, because `makeFontsConf` alone renders a Tailwind sans UI as
  **serif**.
- Keep the WASM bundle lean — avoid heavy deps: `recipe-core` extracts JSON-LD
  with the lightweight `tl` tokenizer (a real parser, not html5ever/scraper,
  which bloat wasm and pull in `getrandom`). Not regex.
- Formatting and CI come from **rainix**. Do NOT add `prettier`,
  `prettier-plugin-*`, or a `.prettierrc` to the frontend — rainix's
  `no-consumer-prettier` pre-commit hook forbids it; the curated bundle
  (exported via `RAINIX_PRETTIER_BUNDLE_DIR` in the dev shell) is the only
  prettier in play.
