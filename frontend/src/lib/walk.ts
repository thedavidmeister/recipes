import { apiFetch } from "./client";
import type { WalkStop } from "./types";

/**
 * Fetch a fresh walk over the corpus (#47) — the `pick` engine.
 *
 * The walk itself runs server-side: the decision logic is the `recipe-walk` Rust
 * crate, and the corpus never leaves the backend. There is deliberately no
 * client-side walk — that would mean shipping the graph and the strategy to the
 * browser, which is the WASM/parse-in-the-client path the app ruled out. So the
 * browser asks and renders; it does not compute.
 *
 * Each call returns a *different* journey — freshness is the whole point — so this
 * is not cached by identity: the caller refetches to wander again.
 *
 * A 401 means the session lapsed since the page loaded; it throws like any other
 * failure so the page shows an error rather than an empty walk. The gate is the
 * server's — this is just a reader.
 */
export async function getWalk(len?: number): Promise<WalkStop[]> {
  const query = len ? `?len=${len}` : "";
  const res = await apiFetch(`/api/walk${query}`);
  if (res.status === 401) throw new Error("Your session has expired.");
  if (!res.ok) throw new Error(`could not walk the corpus (${res.status})`);
  const body = (await res.json()) as { stops: WalkStop[] };
  return body.stops;
}
