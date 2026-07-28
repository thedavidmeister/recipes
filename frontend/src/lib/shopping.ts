import type { Voter } from "./pick";
import type { Ingredient, StructuredMeasure } from "./types";

/**
 * The shopping list's **pure half**: how a recipe becomes a list of lines, and how a
 * ticked line is held. No I/O, no `$env`, nothing that touches a network.
 *
 * That is not tidiness, it is the only place these can live. `$lib/buy` reaches the
 * API client and Turso, and both read `$env/dynamic/public` — so anything imported
 * from there **as a value** drags that read into whatever imports it. In a Storybook
 * bundle that is `undefined` and every story of the component crashes into
 * `Cannot read properties of undefined (reading 'env')`, which the visual fence
 * photographs quite happily. The unit runner has the same problem from the other end:
 * it has no SvelteKit around it on purpose (see `vitest.config.ts`), so a test that
 * imports `$lib/buy` cannot even load. Stories, tests and components take their values
 * from here; `$lib/buy` keeps the fetches.
 *
 * {@link shoppingLines} in particular is the one rule in the app that has to be
 * **identical in two languages**. The browser reads the recipe straight from Turso and
 * renders the list; the server holds the ticks, which are keyed by a line's position
 * *in this list*, and seeds the pantry pre-ticks against the same positions (#156).
 * There is no WASM to share the code through (deliberately — see CLAUDE.md), so the
 * rule is stated here and in `recipe_core::pantry::shopping_names`, and both are pinned
 * by the same case: `shopping.test.ts` here,
 * `drops_unread_lines_so_indices_match_the_shopping_list` there.
 */

/**
 * A ticked line as the screen holds it — the three ways a line can be ticked, in one
 * shape, so no surface can render two of them and forget the third.
 *
 * - `by` set → a person got it, and it wears their colour (#131).
 * - `pantry` set → the plan's kitchen already had it (#156). Nobody's, so no colour;
 *   it says which jar instead.
 * - both `null` → ticked with nothing behind it: a tap still in flight (the server has
 *   not said whose it is yet), or the device-local list, which has no whose at all.
 *
 * Structurally identical to `BuyCheck` in `$lib/buy` minus its index, because it is the
 * same fact: keeping the two in step means the map a component reads is the row the
 * server wrote, with nothing invented in between.
 */
export interface Tick {
  by: Voter | null;
  pantry: string | null;
}

/** A tick with nothing behind it — in flight, or this device's own list. */
export const NOBODY: Tick = { by: null, pantry: null };

/**
 * The lines the checklist shows, in the order it shows them.
 *
 * The list is the structured reading (#11), never the raw measure: the reading is what
 * `buy` renders, and a line with no reading yet is dropped rather than shown raw
 * (`pick` serves read recipes, so a decided one carries readings throughout). Because a
 * dropped line takes no index, this filter *is* the index — a disagreement about which
 * lines count is a disagreement about which row a tick belongs to.
 */
export function shoppingLines(ingredients: Ingredient[]): StructuredMeasure[] {
  return ingredients
    .map((i) => i.structured)
    .filter(
      (s): s is StructuredMeasure => !!s && !!s.item && s.item.trim() !== "",
    );
}
