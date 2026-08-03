import { describe, expect, it } from "vitest";
import { isDecider, isWatching } from "./roster";
import type { Voter } from "./pick";

/**
 * Unit tests for who is deciding a plan and who is only watching it (#180).
 *
 * The predicate is one boolean, and a story cannot pin it: a story is handed
 * `watching` already decided, so a render proves what a watcher *sees*, never who the
 * page calls one. Both of the issue's sub-decisions live in these lines — a seat is
 * asked about, arrival time is not — and both are a `!` away from being wrong in a
 * way nothing else would catch, because the failure is silent in production (a
 * refused vote travels over a socket the server never answers).
 */

const ana: Voter = { telegram_user_id: "4242", username: "ana" };
const bo: Voter = { telegram_user_id: "5150", username: "bo" };
/** A Telegram account need not have a username; identity is the numeric id. */
const nameless: Voter = { telegram_user_id: "9317", username: null };

describe("isDecider", () => {
  it("asks the roster and nothing else", () => {
    expect(isDecider([ana, bo], "4242")).toBe(true);
    expect(isDecider([ana, bo], "9317")).toBe(false);
    expect(isDecider([ana, bo, nameless], "9317")).toBe(true);
  });

  it("claims nobody when the roster or the viewer is unknown", () => {
    // Both are "not yet read", which is not "not on the list".
    expect(isDecider(undefined, "4242")).toBe(false);
    expect(isDecider([ana], undefined)).toBe(false);
    expect(isDecider([], "4242")).toBe(false);
  });
});

describe("isWatching", () => {
  it("says so when a started plan's roster does not hold you", () => {
    // Carol has the link and opened it after the swiping began: `join_lobby` refused
    // her a seat, the lobby read still answers, and she is not in it.
    expect(
      isWatching({ started: true, roster: [ana, bo], viewer: "9317" }),
    ).toBe(true);
  });

  it("leaves a roster member deciding however late they arrive", () => {
    // The issue's first sub-decision. Somebody the host seated before the start (#72)
    // may open the link at any point afterwards; the roster is what was asked, so
    // they still vote. Collapsing "arrived after the start" into "watching" would
    // take the vote off someone who has a seat — and `join_lobby` would re-seat them
    // happily, since its refusal is `started && not on the roster`, never `started`.
    expect(
      isWatching({ started: true, roster: [ana, bo], viewer: "4242" }),
    ).toBe(false);
    expect(
      isWatching({ started: true, roster: [ana, nameless], viewer: "9317" }),
    ).toBe(false);
  });

  it("has nothing to say about a plan that has not started", () => {
    // The lobby seats whoever opens the link, so somebody off the roster here is a
    // moment away from being on it — and the lobby, not the swipe view, is on screen.
    expect(
      isWatching({ started: false, roster: [ana, bo], viewer: "9317" }),
    ).toBe(false);
    expect(
      isWatching({ started: undefined, roster: [ana, bo], viewer: "9317" }),
    ).toBe(false);
  });

  it("waits for the roster and for a viewer before calling anyone a watcher", () => {
    // "Not read yet" must not render as "you are not in this". The socket's lobby
    // frame can set `started` before the HTTP roster read lands, and the session is a
    // cache read that has to resolve first — until both are in, there is no claim to
    // make about anybody.
    expect(
      isWatching({ started: true, roster: undefined, viewer: "9317" }),
    ).toBe(false);
    expect(
      isWatching({ started: true, roster: [ana, bo], viewer: undefined }),
    ).toBe(false);
  });
});
