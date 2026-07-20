import { turso } from "./turso";
import { consensusRef } from "./buy";
import type { CookRecipe } from "./types";

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

  let ingredients: CookRecipe["ingredients"] = [];
  try {
    const parsed = JSON.parse(String(row.ingredients)) as {
      name: string;
      measure: string | null;
    }[];
    ingredients = parsed
      .filter((i) => i.name && i.name.trim() !== "")
      .map((i) => ({ name: i.name, measure: i.measure ?? null }));
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
