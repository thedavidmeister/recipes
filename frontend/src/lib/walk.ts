import { ApiError, apiFetch } from "./client";
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
 * A 401 means the session lapsed since the page loaded; it throws an `ApiError`
 * carrying the status so the caller can re-gate to login rather than looping on a
 * walk that will keep 401ing. The gate is the server's — this is just a reader.
 *
 * `channel` names the pick session this walk feeds (#80): the server bounds the
 * walk to that plan's time cap. Only the channel travels, never the cap itself,
 * so the bound stays server-authoritative — a client cannot deal itself a wider
 * deck than the plan allows.
 */
export async function getWalk(
  len?: number,
  channel?: string,
): Promise<WalkStop[]> {
  const params = new URLSearchParams();
  if (len) params.set("len", String(len));
  if (channel) params.set("channel", channel);
  const query = params.size ? `?${params}` : "";
  const res = await apiFetch(`/api/walk${query}`);
  if (!res.ok) {
    throw new ApiError(
      res.status,
      // "Walk the corpus" is what this call *is*; what failed, from the swiper's
      // side of the screen, is that no more recipes arrived.
      res.status === 401
        ? "You've been signed out. Sign in again to carry on."
        : `Couldn't find more recipes (${res.status}).`,
    );
  }
  const body = (await res.json()) as { stops: WalkStop[] };
  return body.stops;
}
