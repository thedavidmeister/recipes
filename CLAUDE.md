# recipes

Cooking **recipe aggregator** (NOT a CRUD app / CMS — it normalizes existing
public recipe sources into a searchable corpus). Client-heavy design; see the
diagram in [README.md](./README.md).

- **`crates/recipe-core`** — normalization. **`adapters` is the only way into
  the corpus**: an adapter is a source we support (id + host matcher +
  normalizer), `adapters::normalize(url, body)` is the single entry point **and
  the gate**, and it **fails closed** for any host no adapter claims — even one
  serving a valid recipe. It derives the host from the URL itself, never from a
  caller-supplied argument (the two could disagree). schema.org is kept but
  **demoted**: its allowlist is empty, so it claims nothing.
- **`backend/`** — Rust/Axum on **Render**. **ingest** (fetch → derive → store
  both halves), the corpus store, and **`derive`** (a command that rebuilds
  `recipes` from `raw_imports`, no network). Fetching is SSRF-guarded and is
  something ingest **does** — not an endpoint: there is no URL a caller can aim,
  so the backend is not a relay.
- **`frontend/`** — SvelteKit SPA (`adapter-static`) on a **Render static
  site**. It **parses nothing**: it drives ingestion (`POST /api/ingest`), reads
  **Turso** directly (read-only token), and renders. TanStack Query · Bits UI ·
  Tailwind. UI states are declared as **Storybook** stories.
- **Turso** (libSQL/SQLite) is the corpus, in two halves: **`raw_imports`**
  (each recipe's payload as its source gave it) and **`recipes`** (the derived
  view search and browse read). Dev env + CI via **rainix** (`nix develop`).

## The client drives ingestion; the server performs it

The client decides what to look for; the server fetches it, derives recipes, and
stores both halves. **There is no WASM, deliberately.** An in-browser normalizer
only ever existed to parse arbitrary pages the browser fetched itself, and the
corpus no longer ingests arbitrary pages. Once the server fetches, it already
holds the bytes: one normalizer instead of two, nothing to trust from a client,
nothing for a visitor to download, and a source may require a key (which a
public SPA could never hold). **Do not reintroduce client-side parsing** without
undoing that reasoning first.

`recipes` is **derived** — never hand-edit it as a source of truth. Fix the
normalizer and run `derive`; that reaches rows imported before the fix, because
re-fetching is not a recovery plan (sources 502 scrapers, die, and paywall).
**Raw is not an archive**: we only want recipes, so a taxonomy or a browse of
partials leaves no payload.

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

## Auth is mandatory (#25)

**Every API endpoint requires a session.** The only exceptions are `/health` (a
prober holds no session), `/auth/start` + `/auth/poll` (how you get one), and
`/telegram/webhook` (called by Telegram, not a browser — it authenticates with
its own secret). The static SPA shell still loads; it can do nothing until
login.

Since #29 `/api/ingest` **is what a search does**, so gating everything gates
search — deliberately. This is a private cooking app for a group, not a public
search engine.

Auth exists for **identity** (#20 needs a headcount), _not_ to protect the
corpus: nothing writes it from outside and ingest fails closed on an unknown
host, so the surface underneath is already safe. Don't re-argue "gate the
writes" — there are none.

- **The bot logs you in; the site only points at it.** You press Start, the bot
  replies **to you** with a one-time link, and opening it sets the cookie in
  your browser. A bot cannot message someone who has not contacted it first
  (`Forbidden: bot can't initiate conversation with a user`), so "DM me a link"
  is impossible.
- **NEVER add a browser-initiated login.** "The browser starts a login and waits
  for a tap" hands the capability to _redeem_ to whoever **started** it while
  the identity comes from whoever **tapped** — so an attacker starts one, sends
  you the link, and takes your session when you tap. That was built here,
  defended in comments, and reproduced as a full account takeover. Splitting a
  nonce from a poll secret does **not** fix it: that defends "someone saw my
  link", not "the person who sent me this link is the attacker".
- **Accepted cost**: the session lands in whichever browser opens the bot's
  link, so phone-Telegram cannot sign in a desktop. Cross-device transfer _is_
  the attack.
- **The webhook secret is not optional.** Without
  `X-Telegram-Bot-Api-Secret-Token` anyone can POST a forged `/start` claiming
  any Telegram id — a forged login.
- **Identity is the Telegram numeric id, never the username** — usernames are
  mutable and reassignable.
- Secrets are **hashed at rest** and the hash is the lookup key. SHA-256 with no
  KDF is right _here_: they are 256-bit CSPRNG output, so there's nothing to
  brute-force and a KDF would just tax every request.
- **The session is an `HttpOnly` cookie**, never a body field or JS-readable
  token: an XSS can then ride the session but cannot exfiltrate it.
  `SameSite=Lax` suffices **because we own `lehlehleh.com`** — `recipes.` and
  `api.recipes.` are the same site, so the cookie reaches the #20 WebSocket too.
  This is impossible on `onrender.com`: it is on the Public Suffix List, so two
  subdomains of it are different _sites_ and a shared cookie is rejected.
- **CORS is not auth** — it's browser-enforced, `curl` ignores it; the session
  check is the guard. But it must be **explicit**: a credentialed request may
  not be answered `Access-Control-Allow-Origin: *`, and `Any` for origin _or
  methods_ makes tower-http **panic at startup**. Enumerate both.

## Conventions

- Parsing/normalization lives in **`recipe-core` once**, server-side — never
  re-implement it in JS or ship it to the browser.
- **Nothing writes the corpus from outside.** There is no "POST a recipe"
  endpoint: the server stores only what it fetched itself, so the corpus cannot
  be injected. The Turso _write_ token never reaches the browser (it gets a
  read-only token). **SSRF-guard every fetch**: http(s) only; block
  private/loopback/link-local/metadata IPs; timeout; size + redirect caps.
  Ingest additionally fails closed on an unsupported host _before_ fetching, so
  it cannot be used as a general fetch relay.
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
- `recipe-core` extracts JSON-LD with the `tl` tokenizer — **a real parser, not
  regex**. Finding a tag amid comments, quoted attributes and raw script text is
  a parser's job.
- Formatting and CI come from **rainix**. Do NOT add `prettier`,
  `prettier-plugin-*`, or a `.prettierrc` to the frontend — rainix's
  `no-consumer-prettier` pre-commit hook forbids it; the curated bundle
  (exported via `RAINIX_PRETTIER_BUNDLE_DIR` in the dev shell) is the only
  prettier in play.
