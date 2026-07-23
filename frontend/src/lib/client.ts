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

/**
 * Should a failed query be tried again?
 *
 * The backend sleeps. Render's free tier spins a service down after fifteen minutes
 * idle, and waking it takes the better part of a minute — during which a browser's
 * fetch does not return an error code, it simply never completes. Safari reports that
 * as `Load failed`, Chrome as `Failed to fetch`, and neither is a fault worth showing
 * somebody: the service is coming, it is just not there yet.
 *
 * Every query used to be `retry: false`, so a single cold start turned into a dead end
 * — an error page with nothing on it but a breadcrumb, until you thought to reload.
 * That is the first visit of the day, for everyone, and it read as the app being
 * broken.
 *
 * So: a request that never reached the server is retried, patiently enough to cover a
 * cold start. A request the server *answered* is not, because an answer is not a
 * failure to be argued with — a 401 means sign in, a 403 means no, and asking again
 * changes neither. The exceptions are the two codes that explicitly mean "later":
 * timeouts and rate limits.
 */
export function retryTransient(failureCount: number, error: unknown): boolean {
  if (error instanceof ApiError) {
    const worthRepeating =
      error.status >= 500 || error.status === 408 || error.status === 429;
    return worthRepeating && failureCount < 2;
  }
  // No status at all: the request never completed. With TanStack's exponential
  // backoff this spans roughly half a minute, which is what waking a sleeping
  // service costs.
  return failureCount < 5;
}
