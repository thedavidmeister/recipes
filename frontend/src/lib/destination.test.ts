import { describe, expect, it } from "vitest";
import {
  destinationFrom,
  encodeDestination,
  isSameOriginPath,
  HOME,
  MAX_START_PAYLOAD,
} from "./destination";

/**
 * The destination carried through the bot (#206).
 *
 * Two things are under test and only one of them is a guard. The encoding has to
 * survive Telegram's `?start=` alphabet, which is a correctness question; the
 * arrival has to refuse everything that is not a path on this site, which is a
 * security one — the payload is attacker-writable, because anyone at all can
 * `/start` the bot with anything at all.
 *
 * The oracle for the encoding is Telegram's documented limit (≤64 characters of
 * `A–Z a–z 0–9 _ -`) and RFC 4648 §5 base64url, both independent of this code; the
 * expected byte strings below were produced with `Buffer.from(s).toString("base64")`
 * and the two substitutions, not by running the function under test. The oracle for
 * the guard is the set of strings a browser resolves to another origin.
 */

/** A real channel id's shape: `mint_channel_id` is 8 random bytes as hex. */
const CHANNEL = "9f4b2c1d8e7a6053";
const INVITE = `/pick/${CHANNEL}`;
/** `INVITE`, base64url, computed independently of `encodeDestination`. */
const INVITE_PAYLOAD = "L3BpY2svOWY0YjJjMWQ4ZTdhNjA1Mw";

/**
 * A path whose *plain* base64 is `L3BpY2svYWE/YWF+YQ==` — it contains a `/`, a `+`
 * and padding, which are exactly the three things standard base64 produces and
 * Telegram's `?start=` alphabet refuses.
 */
const AWKWARD = "/pick/aa?aa~a";
const AWKWARD_PAYLOAD = "L3BpY2svYWE_YWF-YQ";

describe("encodeDestination", () => {
  it("encodes a pick invite well inside Telegram's ceiling", () => {
    expect(INVITE.length).toBe(22);
    expect(encodeDestination(INVITE)).toBe(INVITE_PAYLOAD);
    expect(INVITE_PAYLOAD.length).toBe(30);
    expect(INVITE_PAYLOAD.length).toBeLessThanOrEqual(MAX_START_PAYLOAD);
  });

  it("only ever spells Telegram's alphabet", () => {
    // Chosen because plain base64 of it is `L3BpY2svYWE/YWF+YQ==` — every character
    // Telegram refuses, in one string: the `/` and `+` of standard base64 and the
    // `=` of padding. base64url is those three substituted away, and this is the
    // only shape of path that can prove all three happen.
    const payload = encodeDestination(AWKWARD);
    expect(payload).toBe(AWKWARD_PAYLOAD);
    expect(payload).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("carries nothing for home, because a bare /start already lands there", () => {
    expect(encodeDestination(HOME)).toBeNull();
  });

  it("carries nothing for a destination that is not ours", () => {
    for (const hostile of [
      "https://evil.example/pick/x",
      "//evil.example",
      "javascript:alert(1)",
      "evil.example",
      "/\\evil.example",
    ]) {
      expect(encodeDestination(hostile)).toBeNull();
    }
  });

  it("carries nothing rather than a truncated path when it will not fit", () => {
    // 48 bytes is the most 64 base64url characters hold; 49 is the first that cannot
    // travel, and a wrong destination is worse than none.
    const fits = "/" + "a".repeat(47);
    const does_not = "/" + "a".repeat(48);
    expect(fits.length).toBe(48);
    expect(encodeDestination(fits)?.length).toBe(MAX_START_PAYLOAD);
    expect(encodeDestination(does_not)).toBeNull();
  });
});

describe("destinationFrom", () => {
  it("round-trips the invite the bot was started from", () => {
    expect(destinationFrom(INVITE_PAYLOAD)).toBe(INVITE);
    expect(destinationFrom(encodeDestination(INVITE))).toBe(INVITE);
    // And the substitutions undo, so a path whose base64 is awkward survives too.
    expect(destinationFrom(AWKWARD_PAYLOAD)).toBe(AWKWARD);
  });

  it("lands home when the bot's link carried no destination at all", () => {
    // Spelled out rather than compared against `HOME`, because "home" is the claim
    // under test: a bare `/start` landed at the site root before #206 and still does.
    expect(HOME).toBe("/");
    expect(destinationFrom(null)).toBe("/");
    expect(destinationFrom(undefined)).toBe("/");
    expect(destinationFrom("")).toBe("/");
  });

  it("discards an absolute URL", () => {
    // `aHR0cHM6…` is `https://evil.example/pick/x`.
    expect(destinationFrom("aHR0cHM6Ly9ldmlsLmV4YW1wbGUvcGljay94")).toBe(HOME);
  });

  it("discards a protocol-relative //host", () => {
    // `Ly9ldmls…` is `//evil.example`, which is another origin despite the slash.
    expect(destinationFrom("Ly9ldmlsLmV4YW1wbGU")).toBe(HOME);
  });

  it("discards a scheme", () => {
    // `amF2YXNj…` is `javascript:alert(1)`.
    expect(destinationFrom("amF2YXNjcmlwdDphbGVydCgxKQ")).toBe(HOME);
    // `bWFpbHRv…` is `mailto:a@b.example`.
    expect(destinationFrom("bWFpbHRvOmFAYi5leGFtcGxl")).toBe(HOME);
  });

  it("discards a backslash a browser would read as a slash", () => {
    // `L1xldmls…` is `/\evil.example`, which resolves as `//evil.example`.
    expect(destinationFrom("L1xldmlsLmV4YW1wbGU")).toBe(HOME);
  });

  it("discards a control character that would assemble //host after stripping", () => {
    // `LwkvL2V2…` is `/<tab>//evil.example`; browsers remove the tab first.
    expect(destinationFrom("LwkvL2V2aWwuZXhhbXBsZQ")).toBe(HOME);
    // `L3BpY2sveAl5` is `/pick/x<tab>y`.
    expect(destinationFrom("L3BpY2sveAl5")).toBe(HOME);
  });

  it("discards a relative path with no leading slash", () => {
    // `cGljay94` is `pick/x` — relative to wherever the browser happens to be.
    expect(destinationFrom("cGljay94")).toBe(HOME);
  });

  it("discards a payload outside Telegram's alphabet", () => {
    // Telegram could not have carried these, so their presence means something else
    // built the link. Note `Ly9ldmlsLmV4YW1wbGU=` — padded — is `//evil.example`,
    // and is refused on the alphabet before it is ever decoded.
    for (const impossible of [
      "Ly9ldmlsLmV4YW1wbGU=",
      "L3BpY2sveA==",
      "a/b",
      "a+b",
      "a b",
      "a.b",
      "a&r=b",
      "a#b",
    ]) {
      expect(destinationFrom(impossible)).toBe(HOME);
    }
  });

  it("discards a payload longer than Telegram can carry", () => {
    // A perfectly good path, spelled in a payload Telegram could not have delivered
    // — 80 characters of base64url for `/aaa…`. The length rule is what refuses it,
    // and it has to be a *decodable* one or the refusal could be the decoder's.
    const long =
      "L2FhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFh";
    expect(long.length).toBeGreaterThan(MAX_START_PAYLOAD);
    expect(long).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(destinationFrom(long)).toBe(HOME);
  });

  it("discards bytes that are not base64url of valid UTF-8", () => {
    // `L3BpY2svgA` is `/pick/` followed by a lone continuation byte (`0x80`). A
    // lenient decoder would hand back `/pick/�`, which passes the same-origin
    // check — so `fatal` decoding is what makes this land home.
    expect(destinationFrom("L3BpY2svgA")).toBe(HOME);
    expect(destinationFrom("gA")).toBe(HOME);
    // A length that is 1 mod 4 cannot be base64 at all, however legal its letters.
    expect(destinationFrom("A")).toBe(HOME);
    expect(destinationFrom("L3BpY2sve")).toBe(HOME);
  });

  it("keeps an ordinary in-site path with a query on it", () => {
    const path = "/kitchens/3?tab=pantry";
    expect(destinationFrom(encodeDestination(path))).toBe(path);
  });
});

describe("isSameOriginPath", () => {
  it("accepts the paths this site actually navigates to", () => {
    for (const ours of [
      "/",
      INVITE,
      "/kitchens",
      "/kitchens/3?tab=pantry",
      "/buy#list",
      // A colon after the first slash is a path segment, not a scheme.
      "/pick/a:b",
    ]) {
      expect(isSameOriginPath(ours)).toBe(true);
    }
  });

  it("refuses every way out of the origin", () => {
    for (const out of [
      "https://evil.example",
      "http://evil.example",
      "//evil.example",
      "///evil.example",
      "javascript:alert(1)",
      "data:text/html,<script>",
      "evil.example",
      "",
      " /pick/x",
      "\\\\evil.example",
      "/\\evil.example",
      "/pick/\\evil.example",
      "/\tevil.example",
      "/\nhttps://evil.example",
      "/pick/x\r\n",
    ]) {
      expect(isSameOriginPath(out)).toBe(false);
    }
  });
});
