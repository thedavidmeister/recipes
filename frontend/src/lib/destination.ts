/**
 * Where a login lands (#206) — the destination, carried out through Telegram and
 * back again.
 *
 * A scan opens the system browser, which has no session, so the invite shows the
 * login screen; signing in used to drop the invite on the floor and land at home,
 * leaving someone authenticated and seated nowhere. So the page worth returning to
 * travels with the login: out as the bot's `?start=` deep-link payload, back as a
 * query parameter beside the secret in the bot's reply.
 *
 * **The destination is not a credential and never becomes one.** Anyone can send the
 * bot any `/start` payload they like, so this decides exactly one thing — where an
 * already-signed-in browser navigates next. It rides *beside* the secret that
 * redeems a session and never inside its verification, so it cannot change who a
 * session belongs to (#25). What it therefore has to be is a **same-origin relative
 * path**, and that is checked here, at the point of use, rather than trusted because
 * of where it came from.
 *
 * ## The encoding
 *
 * Telegram's `?start=` payload is at most 64 characters of `A–Z a–z 0–9 _ -`, which
 * a URL path is not: `/pick/9f4b…` alone is out on the first character. So the path
 * travels **base64url, unpadded** — that alphabet exactly, minus the `=` Telegram
 * would refuse. It costs 4 characters per 3 bytes, so the 64-character ceiling is a
 * 48-byte path, and a pick's invite is 22 of them (`/pick/` plus a 16-hex channel
 * id) — 30 characters encoded, comfortably inside.
 *
 * A base64url payload can spell *anything*, including `//evil.example`, which is
 * precisely why the guard is the relative-path check on the way out and not the
 * encoding. A prefix scheme (`pick_<channel>`) would make an off-origin destination
 * unspellable, and was rejected for it: it would move the guard into the encoding,
 * where the next destination that is not a pick would quietly go unchecked. One
 * general encoding plus one explicit check is the pair that keeps working.
 *
 * A path too long to encode simply does not travel: the bot link falls back to the
 * plain one, the login works exactly as it does today, and the landing is home. A
 * truncated path would be a *wrong* destination, which is worse than none.
 */

/** Telegram's own ceiling on a `?start=` deep-link payload. */
export const MAX_START_PAYLOAD = 64;

/** Where a login lands when nothing said otherwise. */
export const HOME = "/";

/** The alphabet Telegram allows in a `?start=` payload — base64url, without `=`. */
const START_ALPHABET = /^[A-Za-z0-9_-]+$/;

/**
 * Anything a browser strips or collapses before resolving a URL. `\t`, `\n` and `\r`
 * are removed outright, so `/<tab>/evil.example` would resolve as `//evil.example` —
 * a host — after passing a naive "starts with a single slash" test.
 */
const CONTROL = /[\u0000-\u001f\u007f]/;

/**
 * Is this a path on **this** site, and only this site?
 *
 * The whole guard, in four refusals, each of which is a way out of the origin:
 *
 * 1. It must start with `/`. That is what an absolute URL (`https://evil.example`),
 *    a scheme (`javascript:alert(1)`) and a bare host (`evil.example`) all fail.
 * 2. It must not start with `//`. A protocol-relative URL starts with a slash and
 *    still names another host.
 * 3. It must contain no backslash. Browsers read `\` as `/` while resolving, so
 *    `/\evil.example` is `//evil.example` to everything that matters.
 * 4. It must contain no control character, for the reason [`CONTROL`] gives: they
 *    are removed before resolution, so they can assemble rules 2 or 3 out of a
 *    string that reads as neither.
 */
export function isSameOriginPath(path: string): boolean {
  if (!path.startsWith("/")) return false;
  if (path.startsWith("//")) return false;
  if (path.includes("\\")) return false;
  if (CONTROL.test(path)) return false;
  return true;
}

/** base64url, unpadded — Telegram's alphabet exactly. */
function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/**
 * The other direction, or `null` for anything that is not a base64url encoding of
 * valid UTF-8. `atob` is forgiving-base64, so an unpadded payload decodes as it
 * stands and a length that cannot be base64 at all throws.
 *
 * The decoder is **`fatal`**, which is the half that carries weight: a lenient one
 * turns a byte that is not text into U+FFFD, and `/pick/<U+FFFD>` is a perfectly
 * well-formed same-origin path — so a mangled destination would pass the check below
 * rather than fall back to home.
 */
function fromBase64Url(payload: string): string | null {
  const base64 = payload.replace(/-/g, "+").replace(/_/g, "/");
  let binary: string;
  try {
    binary = atob(base64);
  } catch {
    return null;
  }
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

/**
 * The `?start=` payload that would bring a login back to `path`, or `null` when
 * there is nothing to carry.
 *
 * `null` — not an empty string — for the three cases that are all "send today's
 * plain bot link": a path that is not ours to return to, home (which is where a bare
 * `/start` already lands, so saying it again would only be a longer link), and a
 * path too long for Telegram to carry.
 *
 * The same-origin check runs here as well as on arrival. This side is not the guard
 * — the arrival is, because that is where a payload someone else wrote shows up —
 * but a link this site builds should not be the one carrying a destination it would
 * refuse.
 */
export function encodeDestination(path: string): string | null {
  if (!isSameOriginPath(path)) return null;
  if (path === HOME) return null;
  const payload = toBase64Url(path);
  return payload.length <= MAX_START_PAYLOAD ? payload : null;
}

/**
 * Where to land after the cookie is set, from the payload the bot's link carried.
 *
 * **Home unless proven otherwise.** The payload is attacker-writable — anyone can
 * `/start` the bot with anything — so every step is a refusal that falls back to
 * home rather than an error anybody sees: there is nothing for a person to fix, and
 * they are signed in either way.
 */
export function destinationFrom(payload: string | null | undefined): string {
  if (!payload) return HOME;
  if (payload.length > MAX_START_PAYLOAD) return HOME;
  if (!START_ALPHABET.test(payload)) return HOME;
  const path = fromBase64Url(payload);
  if (path === null) return HOME;
  return isSameOriginPath(path) ? path : HOME;
}
