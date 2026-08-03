import { describe, expect, it } from "vitest";
import {
  cookStages,
  formatClock,
  formatEstimate,
  localTimers,
  sharedTimers,
  stepDepths,
} from "./steps";
import type { RunningTimer } from "./session-events";
import type { StructuredStep } from "./types";

/**
 * Unit tests for the pure half of `steps.ts` — the helpers a component calls but
 * cannot declare a story for, because they are arithmetic rather than a render.
 *
 * `formatEstimate` carries the whole honesty contract of the pick card's time badge
 * (#84/#158): unknown must not read as instant, a floor must not read as exact, and
 * an approximation must not read as a floor. All three are one-character mistakes
 * away, so all three are pinned here.
 */

function step(
  id: number,
  seconds: number | null,
  after: number[] = [],
  kind: StructuredStep["kind"] = "cook",
): StructuredStep {
  return { id, text: `step ${id}`, kind, seconds, after };
}

describe("formatEstimate", () => {
  it("shows nothing at all when the estimate is unknown", () => {
    // An un-read recipe is not an instant one. Anything the caller could mistake
    // for "no time needed" has to come back as "say nothing" instead.
    expect(formatEstimate(null)).toBeNull();
    expect(formatEstimate(undefined)).toBeNull();
    expect(formatEstimate(0)).toBeNull();
    expect(formatEstimate(-60)).toBeNull();
    expect(formatEstimate(Number.NaN)).toBeNull();
    expect(formatEstimate(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("marks a partly-untimed estimate as an at-least, never an exact time", () => {
    // Untimed steps count as 0, so the stored number can only be too low and every
    // rendering of it must carry the "+". A format that ever dropped it would
    // present a floor as fact.
    for (const seconds of [30, 60, 900, 1380, 3600, 7500, 9000, 604_800]) {
      expect(formatEstimate(seconds, false)).toMatch(/\+$/);
      expect(formatEstimate(seconds, false)).not.toMatch(/^~/);
    }
  });

  it("defaults to the at-least mark when nothing says the steps are all timed", () => {
    // The safe default is the weaker claim. A caller that forgets the flag — or a
    // row written before `fully_timed` existed — must not be upgraded to "~", which
    // would assert a completeness we have no evidence for.
    expect(formatEstimate(1380)).toBe("23 min+");
    expect(formatEstimate(1380, undefined)).toBe("23 min+");
  });

  it("marks a fully timed estimate as an approximation, never a floor", () => {
    // Every step counted, so the error runs both ways: "~" says about, where "+"
    // would claim the number can only be too low. It is cooking — the durations a
    // source printed are estimates of somebody else's stove too.
    for (const seconds of [30, 60, 900, 1380, 3600, 7500, 9000, 604_800]) {
      expect(formatEstimate(seconds, true)).toMatch(/^~/);
      expect(formatEstimate(seconds, true)).not.toMatch(/\+$/);
    }
  });

  it("changes only the mark, never the words, between the two", () => {
    // The two readings sit side by side down one deck as the corpus is re-read, so
    // the number and its units have to stay identical or the deck looks incoherent.
    for (const seconds of [45, 1380, 7500, 604_800]) {
      const floor = formatEstimate(seconds, false)!;
      const approx = formatEstimate(seconds, true)!;
      expect(approx).toBe(`~${floor.slice(0, -1)}`);
    }
  });

  it("shows nothing under either mark when the estimate is unknown", () => {
    // `fully_timed` is a property of the steps, and a recipe with no timing signal
    // has no number to qualify. "~" on nothing would be worse than "+" on nothing.
    expect(formatEstimate(null, true)).toBeNull();
    expect(formatEstimate(0, true)).toBeNull();
  });

  it("reads under an hour as minutes", () => {
    // 1380 and 3360 are real corpus values — Chicken Handi and a teriyaki bake.
    expect(formatEstimate(60)).toBe("1 min+");
    expect(formatEstimate(1380)).toBe("23 min+");
    expect(formatEstimate(3360)).toBe("56 min+");
    expect(formatEstimate(3599)).toBe("59 min+");
    // 1140 is Gallo pinto once its one untimed step is read — the fully-timed card.
    expect(formatEstimate(1140, true)).toBe("~19 min");
    expect(formatEstimate(1380, true)).toBe("~23 min");
  });

  it("renders the corpus's absurd lower bounds, and why they are the argument", () => {
    // Real stored values, verified against production: Beef Lo Mein (11 steps) holds
    // 10 seconds and a 16-step parcel recipe holds 30, because every step but one
    // counted as nothing. 92 of the 713 timed recipes claim under ten minutes. The
    // "+" is doing real work on these — and the re-read is what makes it stop being
    // needed, one recipe at a time.
    expect(formatEstimate(10)).toBe("10 sec+");
    expect(formatEstimate(30)).toBe("30 sec+");
  });

  it("reads an hour and over as hours, carrying the remainder", () => {
    // Also real corpus values: a Massaman curry stores 7500 and a beef pie 9000.
    // Neither lands on a whole hour — almost nothing does — so the hours branch
    // has to carry minutes rather than fire only on exact multiples. "125 min+"
    // is arithmetic, not an answer to "have I got time for this".
    expect(formatEstimate(3600)).toBe("1 hour+");
    expect(formatEstimate(7200)).toBe("2 hours+");
    expect(formatEstimate(7500)).toBe("2 hours 5 min+");
    expect(formatEstimate(9000)).toBe("2 hours 30 min+");
    expect(formatEstimate(3660)).toBe("1 hour 1 min+");
    // A week-long ferment is in the corpus too. It stays in hours rather than
    // inventing a "days" unit nothing else in the app speaks.
    expect(formatEstimate(604_800)).toBe("168 hours+");
    // The same three shapes under the approximation mark.
    expect(formatEstimate(3600, true)).toBe("~1 hour");
    expect(formatEstimate(7500, true)).toBe("~2 hours 5 min");
    expect(formatEstimate(9000, true)).toBe("~2 hours 30 min");
  });

  it("floors rather than rounds, so it never overstates the bound", () => {
    // 119s is at least 1 minute, not "2 min" — rounding up would inflate a number
    // that is already the optimistic end of the range.
    expect(formatEstimate(119)).toBe("1 min+");
    expect(formatEstimate(1379)).toBe("22 min+");
    expect(formatEstimate(1380.9)).toBe("23 min+");
    expect(formatEstimate(7199)).toBe("1 hour 59 min+");
  });

  it("keeps a sub-minute estimate in seconds instead of losing it", () => {
    // Flooring 45s to minutes would print "0 min+", and dropping it would claim
    // the recipe is un-estimated. It is neither: it is a very short known bound.
    expect(formatEstimate(45)).toBe("45 sec+");
    expect(formatEstimate(59)).toBe("59 sec+");
  });
});

describe("formatClock", () => {
  it("pads seconds and grows to hours", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(65)).toBe("1:05");
    expect(formatClock(1800)).toBe("30:00");
    expect(formatClock(3900)).toBe("1:05:00");
  });

  it("never shows a negative countdown", () => {
    expect(formatClock(-5)).toBe("0:00");
  });
});

describe("stepDepths", () => {
  it("gives steps that share no ordering the same depth", () => {
    const steps = [step(0, null), step(1, null), step(2, null, [0, 1])];
    const depths = stepDepths(steps);
    expect(depths.get(0)).toBe(0);
    expect(depths.get(1)).toBe(0);
    expect(depths.get(2)).toBe(1);
  });

  it("degrades on a malformed cycle rather than hanging", () => {
    const steps = [step(0, null, [1]), step(1, null, [0])];
    expect(() => stepDepths(steps)).not.toThrow();
  });
});

describe("cookStages", () => {
  it("groups cook steps by depth and leaves prep out", () => {
    const steps = [
      step(0, null, [], "prep"),
      step(1, 300, [0]),
      step(2, null, [0]),
      step(3, 60, [1, 2]),
    ];
    const stages = cookStages(steps);
    expect(stages.map((s) => s.steps.map((x) => x.id))).toEqual([[1, 2], [3]]);
  });
});

/**
 * The two timer paths (#208), which have to agree about everything except where the
 * deadline came from.
 *
 * `sharedTimers` is the plan's — instants on a shared timeline, translated through what
 * the server measured *this* device's clock to be doing. `localTimers` is the solo
 * path, whose deadlines this browser wrote off its own clock. A disagreement between
 * them about what "done" means, or about how a remaining second is rounded, would be a
 * cook seeing one thing alone and another thing in company, which is the bug the whole
 * feature exists to remove.
 */
const mel = { telegram_user_id: "5150", username: "mel" };

function running(step: number, deadline: number): RunningTimer {
  return { step, started_at: deadline - 300_000, deadline, started_by: mel };
}

describe("sharedTimers", () => {
  const now = 1_700_000_000_000;

  it("renders the room's deadline on this device's clock", () => {
    // A device a minute fast: the shared deadline is 5 minutes out, and it reads 5
    // minutes out here too, because the offset is added back before the subtraction.
    const timers = sharedTimers(
      [running(7, now + 300_000)],
      60_000,
      now + 60_000,
    );
    expect(timers[7].remaining).toBe(300);
    expect(timers[7].done).toBe(false);
  });

  it("shows the same countdown on two devices whose clocks disagree wildly", () => {
    // The feature, stated as a test: one recorded deadline, two badly-set clocks, one
    // number of seconds left.
    const deadline = now + 300_000;
    const fast = sharedTimers([running(7, deadline)], 60_000, now + 60_000);
    const slow = sharedTimers([running(7, deadline)], -10_000, now - 10_000);
    expect(fast[7].remaining).toBe(slow[7].remaining);
  });

  it("reads done off the deadline, so a finished timer survives everyone being away", () => {
    // Nobody writes "done" anywhere — there is no writer guaranteed to be awake at the
    // moment a deadline passes. A pot whose time went while every browser was closed is
    // still a pot to take off the heat.
    const timers = sharedTimers([running(7, now - 1)], 0, now);
    expect(timers[7].done).toBe(true);
    expect(timers[7].remaining).toBe(0);
  });

  it("never counts below zero", () => {
    expect(sharedTimers([running(7, now - 90_000)], 0, now)[7].remaining).toBe(
      0,
    );
  });

  it("carries who started it, the way a ticked shopping line carries who got it", () => {
    expect(sharedTimers([running(7, now + 1_000)], 0, now)[7].by).toEqual(mel);
  });
});

describe("localTimers", () => {
  const now = 1_700_000_000_000;

  it("reads done off the deadline, exactly as the shared path does", () => {
    const timers = localTimers({ 7: now - 1, 6: now + 300_000 }, now);
    expect(timers[7].done).toBe(true);
    expect(timers[6].done).toBe(false);
    expect(timers[6].remaining).toBe(300);
  });

  it("attributes nothing, because a solo cook has nobody to attribute to", () => {
    // Not an oversight and not a gap to fill in later: there is no plan here, so a name
    // on this timer would be an invented person.
    expect(localTimers({ 7: now + 1_000 }, now)[7].by).toBeUndefined();
  });

  it("rounds a remaining second the same way the shared path does", () => {
    // The two must not disagree by a second, or a cook alone and a cook in company see
    // different numbers for the same recipe.
    const deadline = now + 1_500;
    expect(localTimers({ 7: deadline }, now)[7].remaining).toBe(
      sharedTimers([running(7, deadline)], 0, now)[7].remaining,
    );
  });
});
