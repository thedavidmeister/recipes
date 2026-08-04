import { describe, expect, it, vi } from "vitest";
import { applyFrame } from "./frames";
import type { Cooking, Decided, PickHandlers, ServerMsg } from "./pick";

/**
 * The room's frames, and which handler each reaches (#20, #201).
 *
 * These look like plumbing tests and they are not. An unrecognised frame is dropped
 * **in silence** — deliberately, so one room can serve the pick and `buy` without
 * either writing empty functions for the other's traffic — which means a branch that
 * never fires and a server that never sends are the same thing from in here. Nothing
 * else in the project can tell them apart: `svelte-check` types both alike, the visual
 * fence photographs a screen that is simply never reached, and the socket reconnects
 * happily either way.
 *
 * Since #201 one of those silences is expensive. `decided` is the only frame that ends
 * a pick, and it is the frame a client is sent **on connect** as well as live — so
 * losing it does not slow anybody down, it strands the one person the record exists
 * for: whoever was not watching when the last yes landed.
 */

/** A spy for each handler, so a frame reaching the wrong one is visible. */
function spies() {
  return {
    onTally: vi.fn(),
    onLobby: vi.fn(),
    onVote: vi.fn(),
    onBuy: vi.fn(),
    onLeft: vi.fn(),
    onDecided: vi.fn(),
    onTimePing: vi.fn(),
    onTimeSync: vi.fn(),
    onTimers: vi.fn(),
    onCooking: vi.fn(),
  } satisfies PickHandlers;
}

/** One step's shared countdown, as the room announces it (#208). */
const timer = {
  step: 7,
  started_at: 1_700_000_000_000,
  deadline: 1_700_000_300_000,
  started_by: { telegram_user_id: "5150", username: "mel" },
};

const decided: ServerMsg = {
  type: "decided",
  source: "themealdb",
  id: "52772",
  decided_at: 1_759_000_000,
};

/** The plan is cooking (#211) — the `decided` frame's neighbour one step along the arc. */
const cooking: ServerMsg = {
  type: "cooking",
  started_at: 1_700_000_000_000,
  started_by: { telegram_user_id: "5150", username: "mel" },
};

describe("applyFrame", () => {
  it("hands the decision to onDecided, whole", () => {
    const h = spies();
    applyFrame(decided, h);
    expect(h.onDecided).toHaveBeenCalledTimes(1);
    // The recipe *and* the time it was recorded: the timestamp is the column the
    // server's first-past-the-post guard is written against, so it is part of the
    // fact rather than decoration, and dropping it here would be dropping the
    // difference between "the plan decided" and "the plan is deciding".
    expect(h.onDecided).toHaveBeenCalledWith({
      source: "themealdb",
      id: "52772",
      decided_at: 1_759_000_000,
    } satisfies Decided);
  });

  it("wakes nothing else, so a decision is never read as a vote", () => {
    const h = spies();
    applyFrame(decided, h);
    for (const [name, fn] of Object.entries(h)) {
      if (name === "onDecided") continue;
      expect(fn, `${name} must not see a decided frame`).not.toHaveBeenCalled();
    }
  });

  it("is fine with a client that asked for nothing", () => {
    // `buy` listens to this same room and wants only `onBuy`; a pick that has left
    // its decision handler off must not throw the socket's read.
    expect(() => applyFrame(decided, {})).not.toThrow();
  });

  it("still routes every other frame to its own handler", () => {
    // The regression this half exists for: `decided` was added to a chain of
    // `else if`s, and a frame quietly stealing another's branch is invisible from
    // everywhere except here.
    const h = spies();
    applyFrame({ type: "tally", participants: 2, votes: [] }, h);
    applyFrame(
      {
        type: "lobby",
        deciders: 3,
        started: true,
        seed: 12345,
        created_at: 1_700_000_000,
      },
      h,
    );
    applyFrame(
      { type: "vote", voter: "5150", source: "t", id: "r1", vote: true },
      h,
    );
    applyFrame({ type: "buy", source: "t", id: "r1", checks: [] }, h);
    applyFrame(
      {
        type: "left",
        voter: { telegram_user_id: "5150", username: "mel" },
        ended: false,
      },
      h,
    );

    expect(h.onTally).toHaveBeenCalledWith(2, []);
    // The seed and the plan's birth instant ride the lobby frame (#212) — the two facts
    // a station is computed from. Dropping either here is a room whose music silently
    // falls back to each phone's own dice roll, which looks exactly like it working.
    expect(h.onLobby).toHaveBeenCalledWith(3, true, 12345, 1_700_000_000);
    expect(h.onVote).toHaveBeenCalledWith("5150", "t", "r1", true);
    expect(h.onBuy).toHaveBeenCalledWith("t", "r1", []);
    expect(h.onLeft).toHaveBeenCalledWith(
      { telegram_user_id: "5150", username: "mel" },
      false,
    );
    expect(
      h.onDecided,
      "and none of them is a decision",
    ).not.toHaveBeenCalled();
  });

  it("routes the event framework's three frames, each to its own handler", () => {
    // The two clock frames are the ones this test is really for. Nothing renders them,
    // so a branch that quietly stopped firing would show up only as countdowns that
    // drift apart between two phones — days later, and blamed on anything but this.
    const h = spies();
    applyFrame({ type: "time_ping", server_ms: 1_000 }, h);
    applyFrame({ type: "time_sync", offset_ms: -250, rtt_ms: 40 }, h);
    applyFrame(
      { type: "timers", source: "themealdb", id: "52795", timers: [timer] },
      h,
    );

    expect(h.onTimePing).toHaveBeenCalledWith(1_000);
    expect(h.onTimeSync).toHaveBeenCalledWith(-250, 40);
    expect(h.onTimers).toHaveBeenCalledWith("themealdb", "52795", [timer]);
    // Whole, not merged: the timer list arrives as the server stated it, untouched on
    // the way through, so a screen that replaces its list is replacing it with the room's.
    expect(h.onTimers.mock.calls[0][2][0]).toBe(timer);
    expect(h.onVote, "and none of them is a vote").not.toHaveBeenCalled();
    expect(h.onBuy).not.toHaveBeenCalled();
  });

  it("hands a timers frame to nothing else, so a pot is never read as a decision", () => {
    const h = spies();
    applyFrame(
      { type: "timers", source: "themealdb", id: "52795", timers: [] },
      h,
    );
    for (const [name, fn] of Object.entries(h)) {
      if (name === "onTimers") continue;
      expect(fn, `${name} must not see a timers frame`).not.toHaveBeenCalled();
    }
  });

  it("hands the cook to onCooking, whole", () => {
    // The frame that moves the room to the stove (#211), and the one this side does the
    // least with — which is exactly why it is pinned here. `startCook` navigates nothing
    // on its own, so if this branch stopped firing the tap would look like a dead button
    // for the whole room rather than like one person walking off alone.
    const h = spies();
    applyFrame(cooking, h);
    expect(h.onCooking).toHaveBeenCalledTimes(1);
    // When, and whose — the two facts the record holds. The instant is on the shared
    // timeline, so dropping it would leave a screen with no honest way to say since when.
    expect(h.onCooking).toHaveBeenCalledWith({
      started_at: 1_700_000_000_000,
      started_by: { telegram_user_id: "5150", username: "mel" },
    } satisfies Cooking);
  });

  it("wakes nothing else, so a cook is never read as a decision", () => {
    // The two frames are neighbours in the arc and carry the same shape of fact: one
    // says what the room is having, the other that it is already on the hob. Crossing
    // them would send a plan to `/buy` when it should be at the stove, or the reverse.
    const h = spies();
    applyFrame(cooking, h);
    for (const [name, fn] of Object.entries(h)) {
      if (name === "onCooking") continue;
      expect(fn, `${name} must not see a cooking frame`).not.toHaveBeenCalled();
    }
  });

  it("is fine with a page that is not listening for the cook", () => {
    // The pick page shares this room and has no business in the cook; its handler is
    // simply absent, and the socket's read must not throw over that.
    expect(() => applyFrame(cooking, {})).not.toThrow();
  });

  /** A plan created before plans had a seed says so, rather than arriving as a zero —
   * which would be a perfectly good seed and would put every such plan on one station. */
  it("passes a missing seed through as null", () => {
    const h = spies();
    applyFrame(
      {
        type: "lobby",
        deciders: 1,
        started: false,
        seed: null,
        created_at: 1_600_000_000,
      },
      h,
    );
    expect(h.onLobby).toHaveBeenCalledWith(1, false, null, 1_600_000_000);
  });

  it("drops a frame it does not know rather than throwing", () => {
    // A server ahead of this client — a deploy in flight — must cost a frame, not
    // the socket. Cast because the type says this cannot happen; the wire does not.
    const h = spies();
    expect(() =>
      applyFrame({ type: "something-new" } as unknown as ServerMsg, h),
    ).not.toThrow();
    for (const fn of Object.values(h)) expect(fn).not.toHaveBeenCalled();
  });
});
