# recipes

A cooking **recipe aggregator** — it normalizes recipes from existing public
sources (TheMealDB, and any site that publishes schema.org/Recipe data) into one
shape and builds them into a searchable corpus. It is _not_ a CMS: you don't
author recipes here.

## Architecture

```mermaid
flowchart TD
    subgraph browser["Browser · SvelteKit SPA (static) — Render static site · free"]
        ui["UI — TanStack Query · Bits UI · Tailwind"]
        wasm["recipe-core compiled to WASM<br/>parse + normalize raw bytes, in-browser"]
    end

    subgraph render["Rust · Axum — Render · free, managed"]
        proxy["fetch-proxy<br/>server-side fetch — bypasses CORS / bot walls<br/>SSRF-guarded"]
        write["write-gateway<br/>holds Turso write token · validates + INSERT"]
    end

    turso[("Turso<br/>libSQL / SQLite — the recipe corpus")]
    ext["External sources<br/>TheMealDB (sends CORS) · any recipe site (schema.org JSON-LD)"]

    ui -->|"read corpus · read-only token"| turso
    ui -->|"1 · fetch a URL via proxy"| proxy
    proxy -->|"2 · fetch server-side"| ext
    proxy -->|"3 · raw HTML / JSON"| wasm
    wasm -->|"4 · save normalized recipe"| write
    write -->|"5 · INSERT"| turso
```

**The client does the heavy lifting; the backend is deliberately thin.** The
same Rust crate (`recipe-core`) that could parse on the server is compiled to
**WASM and runs in the browser**, so parsing/normalization happens client-side.
The backend only does the two things a browser _can't_:

1. **Fetch external pages** — browsers can't fetch arbitrary cross-origin sites
   (CORS), and recipe sites actively block scrapers. The backend fetches them
   server-side and returns the raw bytes. (TheMealDB is the exception — it sends
   `Access-Control-Allow-Origin: *`, so the browser calls it directly.)
2. **Write to the database** — the Turso write token must never ship to a public
   browser, so all writes go through the backend write-gateway. Reads use a
   separate read-only token and go direct from the browser.

### Why these choices

| Decision      | Choice                                                           | Why                                                                                                                                                                                                                      |
| ------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Backend host  | **Render** — free, managed, runs a Rust Docker image             | Keeps Rust without a self-managed box, and is **actually free** at our size. Shuttle's free tier ended 2025‑12‑19; Fly.io removed its free allowances in 2024; a VPS (Hetzner) would mean owning host security/patching. |
| Database      | **Turso** — libSQL/SQLite, 5 GB free                             | Managed SQLite: our original SQLite cache design maps over almost 1:1, with no persistent-volume host to run.                                                                                                            |
| Frontend      | **SvelteKit** SPA (`adapter-static`) on a **Render static site** | All logic is client-side, so the frontend is a static bundle. Render static sites are permanently free and never spin down (unlike the free web service), and it keeps the frontend on a host we already run.            |
| Processing    | **Rust → WASM** in the browser                                   | One parser, shared by server and client; keeps compute off the (free, small) backend.                                                                                                                                    |
| Backend scope | fetch-proxy + write-gateway only                                 | The only jobs that genuinely require a server: cross-origin fetches and holding secrets.                                                                                                                                 |

**The infra today is Render + Turso** — that is the whole vendor list, so
nothing else should be described as "already in the stack". That's a statement
of fact, not a ban: adding a service is a decision to take deliberately when
something needs it.

Paths not taken, and why: an all-in-one Cloudflare (Workers + D1 + KV) is
cheaper still, but its free CPU cap forces a TypeScript backend — it would mean
dropping Rust. Vercel's free tier has no always-on server and treats Rust as a
community runtime. Both are reasons they don't host **this Rust backend** — not
verdicts on the vendors.

## Layout

```
crates/recipe-core   shared Rust — models + schema.org + TheMealDB normalize (native + wasm32)
crates/recipe-wasm   wasm-bindgen wrapper → npm package the frontend imports
backend/             Axum fetch-proxy + Turso write-gateway (deploys to Render)
frontend/            SvelteKit SPA — TanStack Query · Bits UI · Tailwind
frontend/.storybook  Storybook — every UI state declared as a story (see below)
flake.nix            rainix `wasm-shell` dev env (Rust + wasm-pack + Node)
```

## Getting started

The dev toolchain comes from [rainix](https://github.com/rainlanguage/rainix)
via Nix — Rust (+ `wasm-pack`), Node, and the shared formatting/CI tooling:

```sh
nix develop
```

- **Shared crate:** `cargo test -p recipe-core`
- **WASM build:** `cargo build -p recipe-core --target wasm32-unknown-unknown`
- **Frontend:** `cd frontend && npm ci && npm run dev`
- **Storybook:** `cd frontend && npm run storybook`

### UI states live in Storybook

Every state a user can see is **declared as a story** rather than reached by
driving the live app — `Pending`, `Error` and `Empty` are impractical to reach
by clicking, and the problem grows as states multiply. Components take their
state as props (e.g. `SearchResults` takes `status: idle|pending|error|ready`),
so the page owns the query and the component owns rendering. Story fixtures
mirror **real** source records; invented ids render as the wrong meal.

## Status

Early. `recipe-core` (shared normalization) is tested and compiles native +
wasm32; the Axum fetch-proxy and Turso write-gateway exist; the SvelteKit SPA
searches TheMealDB end-to-end (fetch → WASM normalize → render) and carries a
Storybook harness. Not yet deployed — see the open issues.

## License

[MIT](./LICENSE)
