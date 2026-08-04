/**
 * The display half of the nutrition reading (#162) — the one calculation the surface
 * is *supposed* to do, and the hedge that goes with it.
 *
 * The corpus stores a **whole-recipe** total (`recipes.kcal`) and the servings it was
 * read as feeding (`recipes.servings`), and deliberately no third column: per serving
 * is a division, not a definition, so a stored one could drift from the two numbers it
 * came from. This module is where that division happens, once, for every surface that
 * ever shows a calorie figure.
 *
 * Pure, and it stays pure — no `$env`, no client, nothing a story cannot import (the
 * `$lib/shopping` precedent).
 */

/**
 * A recipe's calories as a person reads them — **per serving** — or `null` when there
 * is nothing honest to say.
 *
 * ## Per serving, or nothing
 *
 * The whole-recipe total is not the user-facing claim. #162 opens on exactly why: a
 * bare total is ambiguous where it matters most, because 2,400 kcal is a reasonable
 * tray of lasagne and an absurd plate of it — "a number nobody can interpret is worse
 * than no number". So an unread `servings` returns `null` rather than falling back to
 * the total: falling back would hand the reader the ambiguous number precisely when we
 * have the least idea what it means. It is also not divided by an assumed `1`, because
 * "we have not read this" and "this feeds one person" are different facts and one of
 * them is wrong by a factor of four on a tray of lasagne.
 *
 * ## Unknown shows nothing at all
 *
 * `null` kcal is **unread**, never "free" — the same call `formatEstimate` makes about
 * an unread duration, and the same call the column itself makes by being NULL rather
 * than 0 (migration 0023). Food always costs something, so "0 kcal" would be a
 * confident lie about the case we know least about. A garbled or non-positive number
 * takes the same exit: the surface shows nothing rather than something it cannot stand
 * behind.
 *
 * ## The mark says how complete the total is
 *
 * `complete` is `recipes.kcal_complete`, and it picks the mark exactly as
 * `fully_timed` does for time (#84/#158), because the error runs in a different
 * direction in each case:
 *
 * - **`false` — a floor.** At least one ingredient line stated a number nothing could
 *   turn into grams, so it contributed nothing and the real dish costs *at least* this
 *   much. `410 kcal+ a serving` says at-least.
 * - **`true` — an approximation.** Every line that stated a number was counted, so
 *   what is left is ordinary estimation noise: quantities are approximate, "to taste"
 *   has no number, and the oil left in the pan is not eaten. `~410 kcal a serving`
 *   says about. A `+` here would claim the number can only be too low, which the
 *   unmeasurable tail makes untrue in both directions.
 *
 * Neither mark is optional, and that is the point of the pair: #162 asks for an honest
 * hedge and forbids "a confident-looking exact figure", so there is no branch of this
 * function that returns a bare number.
 *
 * ## "a serving" is on screen, not in a tooltip
 *
 * The unit is the whole reason the servings reading exists, so it is rendered, not
 * hovered. Half the readers of this badge are on a phone mid-swipe and will never see
 * a `title`; a badge reading `~410 kcal` beside `23 min+` would leave them to guess
 * whether that is the plate or the tray — the ambiguity the reading was added to
 * remove.
 *
 * The division is floored rather than rounded, for `formatEstimate`'s reason: a `+` is
 * a lower bound and rounding up would overstate it, and using one rule for both marks
 * keeps two cards comparable at a glance down a deck. A serving under 1 kcal is
 * unreachable from real food and returns `null` rather than `0 kcal a serving`.
 */
export function formatCalories(
  kcal: number | null | undefined,
  servings: number | null | undefined,
  complete = false,
): string | null {
  if (kcal == null || !Number.isFinite(kcal) || kcal <= 0) return null;
  if (servings == null || !Number.isFinite(servings) || servings <= 0) {
    return null;
  }
  const each = Math.floor(kcal / servings);
  if (each < 1) return null;
  // The whole difference between the two marks: a floor trails `+`, an approximation
  // leads with `~`. Nothing else about the wording changes, so the two readings stay
  // comparable down a deck of cards — the same shape `formatEstimate` uses for time.
  const open = complete ? "~" : "";
  const close = complete ? "" : "+";
  return `${open}${each} kcal${close} a serving`;
}

/**
 * A plan's calorie range as a person reads it (#213) — the label the lobby's pills and
 * the sentence under them both use, so a chosen pill and the line explaining it can
 * never word the same bound two ways.
 *
 * Either end may be open, and both open is `"Any"` — the same word the time cap uses
 * for the bound that bounds nothing, because it is the same idea and a lobby that
 * called it two things would read as two kinds of control.
 *
 * The unit is deliberately **not** in here. It is constant across every bound (kcal a
 * serving, always), so the surface says it once — in the section's question and in the
 * sentence under the row — rather than four times across a row of pills, which is the
 * opposite call to the time cap, whose units genuinely differ pill to pill.
 *
 * The bounds are inclusive at both ends, the same call the time cap makes (a 30-minute
 * recipe fits a 30-minute cap), so a 500 kcal serving fits both `Up to 500` and
 * `500 to 800`. That is two bounds overlapping at a point, not a recipe in two boxes:
 * these pills choose a bound, they do not sort the corpus.
 */
export function calorieRangeLabel(
  min: number | null | undefined,
  max: number | null | undefined,
): string {
  if (min == null && max == null) return "Any";
  if (min == null) return `Up to ${max}`;
  if (max == null) return `${min} or more`;
  return `${min} to ${max}`;
}

/**
 * What the calorie badge's mark means, spelled out for a reader who can hover.
 *
 * It follows `complete` for the same reason the mark does: telling the reader of a
 * `~410 kcal` card that the number is only a floor is simply wrong about that card,
 * and one sentence for both marks would say the `+` sentence over every `~`. Said in
 * the cook's words (#178) — what is missing from the number, not which column is 0.
 */
export function calorieHint(complete = false): string {
  return complete
    ? "Roughly what one serving costs you — quantities in cooking are estimates, and what stays in the pan isn't eaten."
    : "At least this much a serving — some ingredients here couldn't be weighed, so they counted as nothing.";
}
