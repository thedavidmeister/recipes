import type { Ingredient, StructuredMeasure } from "./types";

/**
 * How a recipe becomes a shopping list — the projection, and nothing else.
 *
 * Its own module because it is the one rule in the app that has to be **identical in
 * two languages**. The browser reads the recipe straight from Turso and renders the
 * list; the server holds the ticks, which are keyed by a line's position *in this
 * list*, and seeds the pantry pre-ticks against the same positions (#156). There is no
 * WASM to share the code through (deliberately — see CLAUDE.md), so the rule is stated
 * here and in `recipe_core::pantry::shopping_names`, and both are pinned by the same
 * case: `shopping.test.ts` here,
 * `drops_unread_lines_so_indices_match_the_shopping_list` there.
 *
 * It sits apart from `$lib/buy` so it can be tested at all: `buy.ts` reaches the API
 * client and Turso, which drag `$env/dynamic/public` in, and the unit runner has no
 * SvelteKit around it on purpose (see `vitest.config.ts`).
 */

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
