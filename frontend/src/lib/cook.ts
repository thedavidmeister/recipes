import { turso } from "./turso";
import { consensusRef } from "./buy";
import type { CookRecipe, Ingredient, StructuredMeasure } from "./types";

/**
 * `cook` (#36) — the picked recipe in full, for following along.
 *
 * The step after `buy`: read the pick's decision (the same consensus recipe) and
 * fetch the whole thing — title, image, ingredients, and the instructions — from
 * Turso, client-direct (the corpus is public). `null` when no pick has decided.
 */
export async function getCookRecipe(): Promise<CookRecipe | null> {
  const ref = consensusRef();
  if (!ref) return null;

  const rs = await turso().execute({
    sql: "SELECT title, image, ingredients, instructions FROM recipes WHERE source = ? AND id = ? LIMIT 1",
    args: [ref.source, ref.id],
  });
  const row = rs.rows[0];
  if (!row) return null;

  // The structured reading (#11) — item, amount, and preparation — never the raw
  // measure. A line with no reading yet is dropped rather than shown raw.
  let ingredients: StructuredMeasure[] = [];
  try {
    const parsed = JSON.parse(String(row.ingredients)) as Ingredient[];
    ingredients = parsed
      .map((i) => i.structured)
      .filter(
        (s): s is StructuredMeasure => !!s && !!s.item && s.item.trim() !== "",
      );
  } catch {
    // Malformed ingredients JSON: still show the recipe + its steps.
  }

  return {
    title: String(row.title),
    image: row.image == null ? null : String(row.image),
    ingredients,
    instructions: String(row.instructions ?? ""),
  };
}
