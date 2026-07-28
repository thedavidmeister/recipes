import { describe, expect, it } from "vitest";
import { shoppingLines } from "./shopping";
import type { Ingredient } from "./types";

/**
 * The shopping list's **indexing**, pinned.
 *
 * `buy_checks` is keyed by a line's position in this list, so this projection decides
 * which row a tick lands on. It is stated twice — here for the browser, which reads the
 * recipe straight from Turso, and in `recipe_core::pantry::shopping_names` for the
 * server, which holds the ticks and seeds the pantry pre-ticks (#156). There is no
 * third place to put it (no WASM, deliberately — see CLAUDE.md), so the two are held
 * together by the same case being asserted on both sides:
 * `drops_unread_lines_so_indices_match_the_shopping_list` in `crates/recipe-core`
 * asserts the same fixture, name for name, index for index.
 *
 * If these two ever drift, a pre-tick lands on the wrong line and somebody is told
 * they already have something they do not.
 */

function ing(name: string, item?: string): Ingredient {
  return {
    name,
    measure: null,
    structured:
      item === undefined
        ? null
        : { item, amount: null, preparation: null, note: null },
  };
}

describe("shoppingLines", () => {
  it("indexes only the lines the checklist shows", () => {
    // The same five lines the Rust test uses, in the same order.
    const lines = shoppingLines([
      ing("2 large onions", "onions"),
      ing("a splash of something"),
      ing("salt", "salt"),
      ing("mystery", "   "),
      ing("1 tbsp olive oil", "olive oil"),
    ]);
    expect(lines.map((l) => l.item)).toEqual(["onions", "salt", "olive oil"]);
    // "salt" is index 1 on the list, not index 2 in the recipe. That shift is the
    // whole reason this is pinned on both sides.
    expect(lines[1].item).toBe("salt");
  });

  it("drops a line with no reading rather than showing it raw", () => {
    // `pick` serves read recipes, so this is a corpus mid-enrichment, not a normal
    // state — but it must not put a nameless row on a shopping list either way.
    expect(shoppingLines([ing("1 (14 oz) can of something")])).toEqual([]);
  });

  it("treats a whitespace-only name as no name", () => {
    // A reading whose item is blank would render as an unlabelled tickbox, and would
    // take an index, pushing every line after it onto the wrong tick.
    expect(shoppingLines([ing("x", ""), ing("y", " \t "), ing("z", "z")])).toEqual([
      { item: "z", amount: null, preparation: null, note: null },
    ]);
  });

  it("keeps an unread recipe's list empty rather than inventing lines", () => {
    expect(shoppingLines([])).toEqual([]);
  });
});
