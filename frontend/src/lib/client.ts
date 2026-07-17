import { env } from "$env/dynamic/public";

/**
 * The one way the SPA talks to the Rust backend.
 *
 * Every `/api` call must carry the session cookie, and the rule that makes that
 * happen — `credentials: "include"` — is easy to forget and silent when omitted:
 * the browser just withholds the cookie and the call 401s. So it lives here, in a
 * single helper both `auth` and `walk` call, rather than being re-typed per module
 * where one copy will eventually drop it. The frontend and backend are different
 * origins (`recipes.` vs `api.recipes.`) even though the same site, which is why
 * the cookie needs including at all.
 */

/**
 * A backend call that failed, carrying the HTTP status so callers can branch on
 * the *kind* of failure (a 401 means the session lapsed) rather than matching on a
 * message.
 */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/** The backend origin, from `PUBLIC_BACKEND_URL` (e.g. `https://api.recipes.…`). */
export function backendUrl(): string {
  const url = env.PUBLIC_BACKEND_URL;
  if (!url) throw new Error("PUBLIC_BACKEND_URL is not set");
  return url.replace(/\/$/, "");
}

/** `fetch` against the backend with the session cookie attached. */
export async function apiFetch(
  path: string,
  init?: RequestInit,
): Promise<Response> {
  return fetch(`${backendUrl()}${path}`, {
    ...init,
    credentials: "include",
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
  });
}
