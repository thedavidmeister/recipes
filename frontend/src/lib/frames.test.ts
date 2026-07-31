import { describe, expect, it, vi } from "vitest";
import { applyFrame } from "./frames";
import type { Decided, PickHandlers, ServerMsg } from "./pick";

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
  } satisfies PickHandlers;
}

const decided: ServerMsg = {
  type: "decided",
  source: "themealdb",
  id: "52772",
  decided_at: 1_759_000_000,
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
    applyFrame(
      { type: "tally", participants: 2, votes: [] },
      h,
    );
    applyFrame({ type: "lobby", deciders: 3, started: true }, h);
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
    expect(h.onLobby).toHaveBeenCalledWith(3, true);
    expect(h.onVote).toHaveBeenCalledWith("5150", "t", "r1", true);
    expect(h.onBuy).toHaveBeenCalledWith("t", "r1", []);
    expect(h.onLeft).toHaveBeenCalledWith(
      { telegram_user_id: "5150", username: "mel" },
      false,
    );
    expect(h.onDecided, "and none of them is a decision").not.toHaveBeenCalled();
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
