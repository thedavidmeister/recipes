import { describe, expect, it } from "vitest";
import {
  pongFor,
  raise,
  toLocal,
  toShared,
  type SessionEvent,
} from "./session-events";

/**
 * The browser half of the time-sync framework (#208).
 *
 * Small functions with a large consequence: the app's rule is that **the initiator's
 * tap is the event** and everybody renders relative to it, corrected for each
 * participant's measured clock drift. Get a sign wrong here and the correction runs the
 * wrong way — a phone a minute fast would land its timer a minute *further* out instead
 * of on the instant it tapped, which looks like a working feature on one device and is
 * wrong for the whole room.
 *
 * The measurement arithmetic itself lives on the server (`events::ClockOffset`), where
 * the round trip is timed; this side answers the ping honestly and translates.
 */

describe("raise", () => {
  it("stamps the initiator's own clock, because that is where the tap happened", () => {
    const tap = 1_700_000_000_000;
    const event: SessionEvent = {
      kind: "timer_start",
      source: "themealdb",
      id: "52795",
      step: 7,
    };
    expect(raise(event, tap)).toEqual({ type: "event", at: tap, event });
  });

  it("puts no duration on the wire — the initiator owns when, never how long", () => {
    const frame = raise(
      { kind: "timer_start", source: "themealdb", id: "52795", step: 7 },
      1,
    );
    // The recipe's own duration is read on the server. A frame with a length in it
    // would let one phone make a 30-minute braise three seconds long for the room, so
    // the shape is pinned rather than merely documented.
    const wire = JSON.stringify(frame);
    expect(wire).not.toContain("seconds");
    expect(wire).not.toContain("duration");
    expect(Object.keys(frame.event).sort()).toEqual([
      "id",
      "kind",
      "source",
      "step",
    ]);
  });

  it("says nothing about who raised it", () => {
    // Identity is the authenticated session. A field here would be a claim this side
    // cannot back, and the server would have to ignore it anyway.
    const frame = raise(
      { kind: "timer_dismiss", source: "s", id: "i", step: 1 },
      1,
    );
    expect(JSON.stringify(frame)).not.toContain("initiator");
    expect(JSON.stringify(frame)).not.toContain("user");
  });

  it("stamps a swipe with the moment the card went, not the moment it is sent", () => {
    // The vote used to be its own frame with no instant on it at all (#209), so what
    // the plan recorded was whenever the row happened to be written. A swipe made in a
    // tunnel and delivered on reconnect is still a swipe made in the tunnel.
    const swiped = 1_700_000_000_000;
    const event: SessionEvent = {
      kind: "vote",
      source: "themealdb",
      id: "52795",
      vote: true,
    };
    expect(raise(event, swiped)).toEqual({ type: "event", at: swiped, event });
  });

  it("stamps a shopping tick the same way, through the same envelope", () => {
    // One envelope for every kind is the whole claim the framework makes, so this is
    // the same assertion as the one above with a different payload — and that it *is*
    // the same assertion is the point.
    const tapped = 1_700_000_012_345;
    const event: SessionEvent = {
      kind: "buy_tick",
      source: "themealdb",
      id: "52795",
      index: 3,
      checked: true,
    };
    expect(raise(event, tapped)).toEqual({ type: "event", at: tapped, event });
    expect(Object.keys(event).sort()).toEqual([
      "checked",
      "id",
      "index",
      "kind",
      "source",
    ]);
  });
});

describe("pongFor", () => {
  it("echoes the server's reading untouched and adds this device's", () => {
    // The echo is what lets the server time the round trip against the send it belongs
    // to; changing it would have an answer timed against the wrong ping and read as a
    // 30-second trip.
    expect(pongFor(1_000, 1_234)).toEqual({
      type: "time_pong",
      server_ms: 1_000,
      client_ms: 1_234,
    });
  });

  it("answers with a wrong clock rather than a corrected one", () => {
    // The point of the exchange is to *measure* the error, so this side must not
    // pre-correct: a client that reported an adjusted reading would measure zero drift
    // and its events would then be normalised by nothing.
    const wrong = 1_700_000_000_000 + 42 * 60 * 1000;
    expect(pongFor(1_000, wrong).client_ms).toBe(wrong);
  });
});

describe("translating between the clocks", () => {
  it("puts a shared instant on this device's clock", () => {
    // offset is `client - server`, so a device 5s fast sees a shared deadline 5s later
    // *by its own clock* — the same real moment as everybody else's.
    expect(toLocal(1_700_000_000_000, 5_000)).toBe(1_700_000_005_000);
    expect(toLocal(1_700_000_000_000, -5_000)).toBe(1_699_999_995_000);
  });

  it("is the exact inverse of putting a local instant on the shared timeline", () => {
    for (const offset of [0, 5_000, -5_000, 42 * 60 * 1000]) {
      const local = 1_700_000_000_123;
      expect(toLocal(toShared(local, offset), offset)).toBe(local);
    }
  });

  it("leaves everything alone when the clocks agree", () => {
    // Not a tautology worth skipping: an unmeasured connection reports an offset of 0,
    // and that has to be the identity or every countdown would move the moment a real
    // measurement replaced it.
    expect(toLocal(1_700_000_000_000, 0)).toBe(1_700_000_000_000);
    expect(toShared(1_700_000_000_000, 0)).toBe(1_700_000_000_000);
  });

  it("makes two badly-wrong devices agree about one shared deadline", () => {
    // The whole feature, in three lines. A deadline recorded on the shared timeline,
    // read by a phone a minute fast and a laptop ten seconds slow: both compute the
    // same *real* moment, so both count down together.
    const deadline = 1_700_000_000_000;
    const phoneNow = 1_699_999_940_000 + 60_000; // its clock, a minute fast
    const laptopNow = 1_699_999_940_000 - 10_000; // its clock, ten seconds slow
    const phoneLeft = toLocal(deadline, 60_000) - phoneNow;
    const laptopLeft = toLocal(deadline, -10_000) - laptopNow;
    expect(phoneLeft).toBe(laptopLeft);
    expect(phoneLeft).toBe(60_000);
  });
});
