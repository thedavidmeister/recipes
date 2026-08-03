import { env } from "$env/dynamic/public";
import { apiFetch } from "./client";
import { encodeDestination } from "./destination";
import type { User } from "./types";

/**
 * Telegram magic-link auth (#25). Auth is **mandatory**: every API call needs a
 * session, search included, because since #29 a search *is* an ingest.
 *
 * Nothing here ever holds the session token. It lives in an `HttpOnly` cookie the
 * browser sets and sends on its own, so this module cannot read it even to check
 * — which is the point: an XSS can ride the session but cannot steal it. `me()`
 * is therefore the only way to ask "am I logged in?", and the server answers.
 *
 * There is deliberately **no way to start a login from here**. The bot mints the
 * secret for whoever messages it and sends the link to that person's chat. A
 * browser-initiated flow would hand the redeeming capability to whoever started
 * it while the identity came from whoever tapped — which is a full account
 * takeover, and was one, before this design replaced it.
 *
 * The cross-origin fetch (and the `credentials: "include"` that carries the
 * cookie) lives in `./client`, shared with every other `/api` caller.
 */

/**
 * The bot to send people to, e.g. `lehlehlehbot`. Public by nature.
 *
 * `returnTo` is the page the login was reached from, and it makes the link a deep
 * link: `?start=<payload>` comes back to the bot as the message `/start <payload>`,
 * so the destination travels out with the person and back in the link the bot
 * replies with (#206). A scan lands in a browser with no session, and without this
 * the invite is simply lost — you sign in and arrive at home, seated nowhere.
 *
 * It carries **where**, never **who**. The session is still minted for whoever
 * pressed Start in their own Telegram and delivered only to their chat (#25);
 * nothing here starts a login, and this argument could not if it tried.
 *
 * A destination that will not fit, or that is not a path on this site, is simply not
 * carried — the plain bot link is the answer, and the landing is home.
 */
export function botLink(returnTo?: string): string {
  const bot = env.PUBLIC_TELEGRAM_BOT;
  if (!bot) throw new Error("PUBLIC_TELEGRAM_BOT is not set");
  const payload = returnTo ? encodeDestination(returnTo) : null;
  return payload
    ? `https://t.me/${bot}?start=${payload}`
    : `https://t.me/${bot}`;
}

/**
 * Who is logged in, or `null` if nobody.
 *
 * A 401 is not an error here — it is the expected answer for a visitor without a
 * session, and the SPA's boot question. Anything else is a real failure and
 * throws, so a broken backend cannot masquerade as "logged out" and bounce
 * someone to a login they do not need.
 */
export async function me(): Promise<User | null> {
  const res = await apiFetch("/api/me");
  if (res.status === 401) return null;
  if (!res.ok)
    throw new Error(`Couldn't check whether you're signed in (${res.status}).`);
  return (await res.json()) as User;
}

/**
 * Redeem the secret from the bot's link. The session arrives as a cookie on this
 * response, so there is nothing to store.
 */
export async function completeLogin(c: string): Promise<void> {
  const res = await apiFetch("/api/auth/complete", {
    method: "POST",
    body: JSON.stringify({ c }),
  });
  if (res.status === 401)
    throw new Error("That link is expired or already used.");
  if (!res.ok) throw new Error(`Couldn't sign you in (${res.status}).`);
}

/** Drop the session, server-side and in the browser. */
export async function logout(): Promise<void> {
  const res = await apiFetch("/api/auth/logout", { method: "POST" });
  if (!res.ok) throw new Error(`Couldn't sign you out (${res.status}).`);
}
