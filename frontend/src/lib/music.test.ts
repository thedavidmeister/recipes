import { readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CONVERGE_S,
  IN_SYNC_S,
  MAX_NUDGE,
  POOLS,
  SEEK_S,
  expectedPosition,
  planChannel,
  planSection,
  poolFor,
  reconcile,
  type RoomTrack,
} from "./music";

/**
 * The room's soundtrack (#212) and a device keeping itself in it (#214).
 *
 * Small functions with a large consequence, and none of it visible on one screen: every
 * bug this can have looks like a working feature on the device you are holding. A sign
 * error in the position arithmetic puts a phone whose clock is fast further into the
 * song rather than exactly where everybody else is; a nudge that runs the wrong way
 * races away from the room the harder it tries; a reconcile that answers "seek" where it
 * should answer "advance" jumps to a position past the end of a track forever. All of
 * those pass a screenshot, a `svelte-check` and a live poke on one laptop.
 *
 * The room's *choice* of track is not here — it is the server's (`backend/src/music.rs`),
 * so that the shuffle and the no-back-to-back rule are one for the room rather than one
 * per phone, and so that no client can name a URL every phone would load.
 */

/** The room, mid-track. */
const room: RoomTrack = {
  section: "cook",
  track: "/music/cook-2.mp3",
  started_at: 1_700_000_000_000,
};

/** A track long enough that nothing below runs off the end of it by accident. */
const LONG = 240;

describe("planChannel", () => {
  it("reads the plan out of the pick's own URL", () => {
    expect(planChannel("/pick/ab12cd34", undefined)).toBe("ab12cd34");
    // Percent-encoded on the way in (`PickClient.url`), so read back the same way.
    expect(planChannel("/pick/a%2Fb", undefined)).toBe("a/b");
  });

  it("carries the plan into buy, cook and joy with the stashed decision", () => {
    for (const path of ["/buy", "/cook", "/joy"]) {
      expect(planChannel(path, "ab12cd34")).toBe("ab12cd34");
    }
  });

  /**
   * **The whole solo story is this null.** No channel means no socket, no reconcile loop
   * and the lone device's random pick untouched — so every one of these is a page that
   * must never open a room, and a widening of this function is how solo traffic would
   * appear. `/pick` with nothing after it is the page that *starts* a plan, and a
   * decision stashed by a build from before plans carried a channel has none either.
   */
  it("has no room to open where there is no plan", () => {
    expect(planChannel("/kitchens", "ab12cd34")).toBeNull();
    expect(planChannel("/kitchens/k1/pantry", "ab12cd34")).toBeNull();
    expect(planChannel("/", "ab12cd34")).toBeNull();
    expect(planChannel("/health", "ab12cd34")).toBeNull();
    expect(planChannel("/pick", "ab12cd34")).toBeNull();
    expect(planChannel("/buy", undefined)).toBeNull();
    expect(planChannel("/cook", null)).toBeNull();
  });
});

describe("planSection", () => {
  it("names the four legs of a meal, and nothing else", () => {
    expect(planSection("/pick/ab12")).toBe("pick");
    expect(planSection("/buy")).toBe("buy");
    expect(planSection("/cook")).toBe("cook");
    expect(planSection("/joy")).toBe("joy");
    // Kitchens has music and is not something a room does together.
    expect(planSection("/kitchens")).toBeNull();
    expect(planSection("/")).toBeNull();
  });
});

describe("POOLS", () => {
  /**
   * Every track a lone device can play is a file that is actually served. Its
   * counterpart on the server (`every_track_the_room_can_name_is_a_file_that_exists`)
   * guards the room's pools against the same rename, which is otherwise silence that
   * looks exactly like the bug all this fixes.
   */
  it("names only files that exist under static/", () => {
    const root = new URL("../../static", import.meta.url).pathname;
    const present = new Set<string>();
    const walk = (dir: string, prefix: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.isDirectory())
          walk(join(dir, entry.name), `${prefix}${entry.name}/`);
        else present.add(`/${prefix}${entry.name}`);
      }
    };
    walk(root, "");
    for (const [route, pool] of Object.entries(POOLS)) {
      for (const track of pool) {
        expect(present.has(track), `${route} names ${track}`).toBe(true);
      }
    }
  });

  it("serves a route with no music an empty pool rather than someone else's", () => {
    expect(poolFor("/health")).toEqual([]);
    expect(poolFor("/joy")).toEqual([]);
    expect(poolFor("/cook").length).toBeGreaterThan(1);
  });
});

describe("expectedPosition", () => {
  /**
   * **The hand-computed fixture.** A track started at 1_700_000_000_000 on the shared
   * timeline; this device's clock has been measured 5,000ms fast and currently reads
   * 1_700_000_042_000. Its own reading is therefore 42s past the start, but 5 of those
   * seconds are its clock being wrong, so the room is **37s** in — and that is where this
   * device must play, not 42.
   */
  it("takes this device's measured drift off its own clock", () => {
    expect(expectedPosition(1_700_000_000_000, 5_000, 1_700_000_042_000)).toBe(
      37,
    );
  });

  /** The same tap, on a phone a minute *slow*: it reads a smaller `now`, so its offset
   * is negative and comes off as an addition. Both phones answer the same position,
   * which is the whole point of measuring the clock at all. */
  it("agrees between two phones whose clocks disagree", () => {
    const fast = expectedPosition(1_700_000_000_000, 60_000, 1_700_000_090_000);
    const slow = expectedPosition(
      1_700_000_000_000,
      -60_000,
      1_699_999_970_000,
    );
    expect(fast).toBe(30);
    expect(slow).toBe(30);
  });

  it("is zero at the instant the track starts, and negative just before it", () => {
    expect(expectedPosition(1_700_000_000_000, 0, 1_700_000_000_000)).toBe(0);
    expect(expectedPosition(1_700_000_000_000, 0, 1_699_999_999_500)).toBe(
      -0.5,
    );
  });
});

describe("reconcile", () => {
  /** A device playing the room's track, `at` seconds in by its own element. */
  const playing = (
    at: number,
    track: string | null = room.track,
    duration = LONG,
  ) => ({ track, currentTime: at, duration });

  /** The deadband, at its edge — measured from zero so the numbers are exact rather
   * than an artefact of adding two decimals. `IN_SYNC_S` itself still holds: the
   * correction stops *at* the threshold, which is what keeps it from hunting around
   * zero forever. */
  it("holds when the difference is inaudible", () => {
    expect(reconcile(room, playing(0), 0)).toEqual({ kind: "hold" });
    expect(reconcile(room, playing(IN_SYNC_S), 0)).toEqual({ kind: "hold" });
    expect(reconcile(room, playing(-IN_SYNC_S), 0)).toEqual({ kind: "hold" });
    expect(reconcile(room, playing(IN_SYNC_S * 2), 0).kind).toBe("nudge");
  });

  /**
   * **Ahead of the room ⇒ slow down.** The sign is the whole branch: run it the other
   * way and every device races further from the room the harder it tries to catch up,
   * which is a bug that sounds like nothing at all on the device you are testing on.
   */
  it("nudges slower when this device is ahead, and faster when behind", () => {
    const ahead = reconcile(room, playing(30.2), 30);
    const behind = reconcile(room, playing(29.8), 30);
    expect(ahead.kind).toBe("nudge");
    expect(behind.kind).toBe("nudge");
    // Proportional, so it eases off as it converges instead of overshooting and hunting
    // back — 0.2s out of a 10s convergence is 2% under normal speed.
    expect(ahead.kind === "nudge" && ahead.rate).toBeCloseTo(
      1 - 0.2 / CONVERGE_S,
      6,
    );
    expect(behind.kind === "nudge" && behind.rate).toBeCloseTo(
      1 + 0.2 / CONVERGE_S,
      6,
    );
    expect(ahead.kind === "nudge" && ahead.rate).toBeLessThan(1);
    expect(behind.kind === "nudge" && behind.rate).toBeGreaterThan(1);
  });

  /** The nudge is capped where a listener would start to hear it as a change of pitch
   * rather than a change of when. */
  it("never nudges past the inaudible cap", () => {
    const near = reconcile(room, playing(30 + SEEK_S - 0.001), 30);
    expect(near).toEqual({ kind: "nudge", rate: 1 - MAX_NUDGE });
  });

  /**
   * **The nudge/seek split**, exactly at the threshold. Below it the repair is inaudible;
   * at it and above, the drift is a stall or a wake rather than drift, and a nudge would
   * leave two phones in one kitchen audibly out of phase for the ~17 checks it would take.
   */
  it("seeks at the threshold and nudges just under it", () => {
    expect(reconcile(room, playing(30 + SEEK_S), 30)).toEqual({
      kind: "seek",
      position: 30,
    });
    expect(reconcile(room, playing(30 - SEEK_S), 30)).toEqual({
      kind: "seek",
      position: 30,
    });
    expect(reconcile(room, playing(30 + SEEK_S - 0.0001), 30).kind).toBe(
      "nudge",
    );
  });

  /** A phone that slept and woke a whole track later is a seek, not a slow crawl. */
  it("seeks a device that was asleep back into the room", () => {
    expect(reconcile(room, playing(12), 118)).toEqual({
      kind: "seek",
      position: 118,
    });
  });

  /**
   * **A missed rollover is desync too.** The room is on a different track — the phone
   * was asleep or disconnected when it changed — so load the current one at the current
   * position. This is the same rehydrate story as every other piece of plan state (#202),
   * and it is ranked first because no position on the wrong song means anything.
   */
  it("loads the room's track when this device is on another one", () => {
    expect(reconcile(room, playing(90, "/music/cook-1.mp3"), 12)).toEqual({
      kind: "load",
      track: "/music/cook-2.mp3",
      position: 12,
    });
    // Including from silence, which is what a device that has just arrived is.
    expect(reconcile(room, playing(0, null), 12).kind).toBe("load");
  });

  /**
   * **Past the end is not a seek.** The room's timeline has run off this track — every
   * device was asleep through the end of it, or the frame that would have moved it on
   * was lost — so there is no position to converge on and the answer is to move the room.
   * Seeking here would jump to a place the track does not have, every second, forever.
   */
  it("asks the room to move on when its track has ended", () => {
    expect(reconcile(room, playing(LONG, room.track, LONG), LONG)).toEqual({
      kind: "advance",
    });
    expect(reconcile(room, playing(LONG, room.track, LONG), LONG + 60)).toEqual(
      {
        kind: "advance",
      },
    );
  });

  /** A duration that is not known yet is not an answer, so it is not read as one: the
   * element is still loading metadata and the next check has the number. */
  it("does not call a track ended on a duration it does not have", () => {
    expect(reconcile(room, playing(0, room.track, NaN), 400).kind).toBe("seek");
  });

  /** A rollover instant a moment in the future plays from the top rather than seeking
   * to a place no track has. */
  it("never asks for a negative position", () => {
    expect(reconcile(room, playing(3), -0.4)).toEqual({
      kind: "seek",
      position: 0,
    });
    expect(reconcile(room, playing(0, null), -0.4)).toEqual({
      kind: "load",
      track: room.track,
      position: 0,
    });
  });
});
