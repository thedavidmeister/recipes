import { env } from "$env/dynamic/public";
import type { LoginStart, PollResult, User } from "./types";

/**
 * Telegram magic-link auth (#25). Auth is **mandatory**: every API call needs a
 * session, search included, because since #29 a search *is* an ingest.
 *
 * Nothing here ever holds the session token. It lives in an `HttpOnly` cookie
 * the browser sets and sends on its own, so this module cannot read it even to
 * check — which is the point: an XSS can ride the session but cannot steal it.
 * `me()` is therefore the only way to ask "am I logged in?", and the honest
 * answer comes from the server.
 */
function backend(): string {
  const url = env.PUBLIC_BACKEND_URL;
  if (!url) throw new Error("PUBLIC_BACKEND_URL is not set");
  return url.replace(/\/$/, "");
}

/**
 * Every call carries cookies. Without `credentials: "include"` the browser
 * withholds the session on a cross-origin request and everything 401s — the
 * frontend and backend are different origins (`recipes.` vs `api.recipes.`) even
 * though they are the same *site*.
 */
async function api(path: string, init?: RequestInit): Promise<Response> {
  return fetch(`${backend()}${path}`, {
    ...init,
    credentials: "include",
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
  });
}

/**
 * Who is logged in, or `null` if nobody.
 *
 * A 401 is not an error here — it is the expected answer for a visitor without a
 * session, and the SPA's boot question. Anything else is a real failure and
 * throws, so a broken backend cannot masquerade as "logged out" and quietly
 * bounce someone to a login they do not need.
 */
export async function me(): Promise<User | null> {
  const res = await api("/api/me");
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`could not check session (${res.status})`);
  return (await res.json()) as User;
}

/**
 * Begin a login: returns the deep link to show, and the secret that redeems it.
 *
 * The two are deliberately different values. The link is **shareable** — it is
 * meant to be tapped, screenshotted, and (for #20) posted into a group chat — so
 * it cannot also be what claims the session. `pollSecret` never leaves this tab.
 */
export async function startLogin(): Promise<LoginStart> {
  const res = await api("/api/auth/start", { method: "POST" });
  if (!res.ok) throw new Error(`could not start login (${res.status})`);
  const json = await res.json();
  return {
    link: json.link as string,
    pollSecret: json.poll_secret as string,
    expiresAt: json.expires_at as number,
  };
}

/**
 * Ask whether the link has been tapped yet.
 *
 * On `ready` the server has already set the session cookie on this response —
 * there is no token in the body to store, and nothing for this code to do but
 * stop polling.
 */
export async function pollLogin(pollSecret: string): Promise<PollResult> {
  const res = await api("/api/auth/poll", {
    method: "POST",
    body: JSON.stringify({ poll_secret: pollSecret }),
  });
  if (!res.ok) throw new Error(`login check failed (${res.status})`);
  return (await res.json()) as PollResult;
}

/** Drop the session, server-side and in the browser. */
export async function logout(): Promise<void> {
  const res = await api("/api/auth/logout", { method: "POST" });
  if (!res.ok) throw new Error(`logout failed (${res.status})`);
}
