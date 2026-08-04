import { describe, expect, it } from "vitest";
import { calorieHint, calorieRangeLabel, formatCalories } from "./nutrition";

/**
 * Unit tests for the pick card's calorie badge (#162) — the arithmetic a story cannot
 * declare, because a story is a picture of one recipe and this is the rule applied to
 * all of them.
 *
 * The numbers here are deliberately synthetic and that is the *right* place for
 * synthetic numbers: this pins a division and a pair of marks, not a claim about any
 * dish. A fixture asserting "Chicken Handi is 410 kcal a serving" would be a claim
 * about a real record, and those come from the corpus sample or not at all (#157).
 *
 * Three honesty rules ride on this function and each is one character away from being
 * broken, so each is pinned:
 *
 * 1. unread must not read as free,
 * 2. a total with no servings must not read as a per-serving figure,
 * 3. a floor must not read as an estimate, nor an estimate as a floor.
 */
describe("formatCalories", () => {
  it("shows nothing at all when the total is unread", () => {
    // Food always costs something, so an absent reading is a gap in our reading, not
    // a dish with no calories. Anything a reader could take for "free" has to come
    // back as "say nothing" — the same call `formatEstimate` makes about time, and the
    // same call the column makes by being NULL rather than 0.
    expect(formatCalories(null, 4)).toBeNull();
    expect(formatCalories(undefined, 4)).toBeNull();
    expect(formatCalories(0, 4)).toBeNull();
    expect(formatCalories(-800, 4)).toBeNull();
    expect(formatCalories(Number.NaN, 4)).toBeNull();
    expect(formatCalories(Number.POSITIVE_INFINITY, 4)).toBeNull();
  });

  it("shows nothing when there is no servings reading to divide by", () => {
    // The failure this exists to prevent: falling back to the whole-recipe total. That
    // number is ambiguous exactly where it matters — 2,400 kcal is a reasonable tray of
    // lasagne and an absurd plate of it — so #162 calls it worse than no number.
    expect(formatCalories(2400, null)).toBeNull();
    expect(formatCalories(2400, undefined)).toBeNull();
    expect(formatCalories(2400, 0)).toBeNull();
    expect(formatCalories(2400, -2)).toBeNull();
    expect(formatCalories(2400, Number.NaN)).toBeNull();
  });

  it("never quietly assumes the recipe feeds one", () => {
    // "We have not read this" and "this feeds one person" are different facts, and the
    // difference is a factor of four on a tray of lasagne. An unread count that fell
    // back to 1 would render the total while looking like a per-serving figure — the
    // worst of both, and indistinguishable on screen from a real single serving.
    expect(formatCalories(2400, null)).toBeNull();
    expect(formatCalories(2400, 1)).toBe("2400 kcal+ a serving");
  });

  it("marks an incomplete total as an at-least, never an exact figure", () => {
    // A line stating a number nothing could weigh counted as nothing, so the dish can
    // only cost more. Every rendering must carry the "+"; dropping it presents a floor
    // as fact.
    for (const kcal of [40, 400, 1640, 2400, 9000]) {
      expect(formatCalories(kcal, 4, false)).toMatch(/\+ a serving$/);
      expect(formatCalories(kcal, 4, false)).not.toMatch(/^~/);
    }
    expect(formatCalories(1640, 4, false)).toBe("410 kcal+ a serving");
  });

  it("defaults to the at-least mark when nothing says the total is complete", () => {
    // The safe default is the weaker claim, as it is for time: a caller that forgets
    // the flag must not be upgraded to "~", which asserts a completeness we have no
    // evidence for.
    expect(formatCalories(1640, 4)).toBe("410 kcal+ a serving");
    expect(formatCalories(1640, 4, undefined)).toBe("410 kcal+ a serving");
  });

  it("marks a complete total as an approximation, never a floor", () => {
    // Every line that stated a number was counted, so what is left runs both ways:
    // quantities are approximate and the oil left in the pan is not eaten. "~" says
    // about, where "+" would claim the number can only be too low.
    for (const kcal of [40, 400, 1640, 2400, 9000]) {
      expect(formatCalories(kcal, 4, true)).toMatch(/^~/);
      expect(formatCalories(kcal, 4, true)).not.toMatch(/\+/);
    }
    expect(formatCalories(1640, 4, true)).toBe("~410 kcal a serving");
  });

  it("says the serving on screen, in both marks", () => {
    // The unit is the whole reason the servings reading exists (#162), and a swiper on
    // a phone never sees a `title`. A badge reading "~410 kcal" beside "23 min+" would
    // leave them guessing whether that is the plate or the tray.
    expect(formatCalories(1640, 4, true)).toContain("a serving");
    expect(formatCalories(1640, 4, false)).toContain("a serving");
  });

  it("divides, and floors the division rather than rounding it up", () => {
    // Per serving is the surface's division (there is no third stored column), and it
    // is floored for `formatEstimate`'s reason: a "+" is a lower bound and rounding it
    // up would overstate it. One rule for both marks keeps two cards comparable at a
    // glance down a deck.
    expect(formatCalories(1045, 4, true)).toBe("~261 kcal a serving");
    expect(formatCalories(1045, 4, false)).toBe("261 kcal+ a serving");
    expect(formatCalories(999, 2, true)).toBe("~499 kcal a serving");
  });

  it("shows nothing rather than a serving that costs nothing", () => {
    // Only reachable from a nonsense pair (more servings than kcal), and it lands on
    // the one string this badge must never print: a plate of food with no calories in
    // it. It takes the unread exit instead.
    expect(formatCalories(3, 4, true)).toBeNull();
    expect(formatCalories(1, 100, false)).toBeNull();
  });
});

describe("calorieHint", () => {
  it("does not tell a complete reading it is only a floor", () => {
    // One sentence for both marks would say the "+" sentence over every "~" — the bug
    // #158 fixed in the time badge's tooltip, arriving again in a new column.
    expect(calorieHint(true)).not.toContain("At least");
    expect(calorieHint(false)).toContain("At least");
  });

  it("defaults to the weaker claim, like the mark it explains", () => {
    expect(calorieHint()).toBe(calorieHint(false));
  });
});

/**
 * The lobby's calorie range label (#213) — the one place a plan's bound is put into
 * words, read by both the pill and the sentence under the row, so the two can never
 * word the same bound differently.
 */
describe("calorieRangeLabel", () => {
  it("calls an unbounded plan Any, the same word the time cap uses", () => {
    // A lobby that called the bound-that-bounds-nothing two different things across
    // two rows would read as two kinds of control. It is one idea.
    expect(calorieRangeLabel(null, null)).toBe("Any");
    expect(calorieRangeLabel(undefined, undefined)).toBe("Any");
  });

  it("says which end is open when only one is", () => {
    // One open end still bounds — the walk enforces exactly this asymmetry — so the
    // label has to name the end that is doing the work and not imply the other.
    expect(calorieRangeLabel(null, 500)).toBe("Up to 500");
    expect(calorieRangeLabel(800, null)).toBe("800 or more");
  });

  it("names both ends when both are stated", () => {
    expect(calorieRangeLabel(500, 800)).toBe("500 to 800");
    // A range of one value is a range, not an error: the ends are inclusive.
    expect(calorieRangeLabel(702, 702)).toBe("702 to 702");
  });

  it("leaves the unit to the surface, which says it once", () => {
    // The unit never varies (kcal a serving, always), so a row of four pills would
    // repeat it four times. The question above the row and the note below it carry
    // it; the label carries only the numbers.
    expect(calorieRangeLabel(500, 800)).not.toContain("kcal");
    expect(calorieRangeLabel(null, 500)).not.toContain("serving");
  });
});
