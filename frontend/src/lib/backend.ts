import { env } from "$env/dynamic/public";
import type { Recipe } from "./types";

/**
 * The backend does the two things a browser can't: fetch cross-origin pages and
 * hold the Turso *write* token. Reads go direct to Turso; writes come here.
 */
function backend(): string {
  const url = env.PUBLIC_BACKEND_URL;
  if (!url) throw new Error("PUBLIC_BACKEND_URL is not set");
  return url.replace(/\/$/, "");
}

/**
 * A recipe is only worth storing once it carries the parts a corpus is for.
 * TheMealDB's `filter.php` (category browse) returns header fields only, and
 * the write-gateway's upsert overwrites every column — so saving a partial over
 * an existing full record would blank its ingredients and instructions. Saving
 * is gated on this rather than on the caller remembering.
 */
export function isComplete(recipe: Recipe): boolean {
  return recipe.instructions.trim().length > 0 && recipe.ingredients.length > 0;
}

/** Save one recipe to the corpus. Rejects partials — see `isComplete`. */
export async function saveRecipe(recipe: Recipe): Promise<void> {
  if (!isComplete(recipe)) {
    throw new Error(`refusing to save a partial recipe: ${recipe.source}/${recipe.id}`);
  }
  const res = await fetch(`${backend()}/api/recipes`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(recipe),
  });
  if (!res.ok) {
    throw new Error(`save failed (${res.status}) for ${recipe.source}/${recipe.id}`);
  }
}

/**
 * Save every complete recipe, skipping partials. Returns how many were stored.
 * One failure doesn't sink the batch — persisting the corpus is a side effect
 * of browsing, so it must never break the render.
 */
export async function saveRecipes(recipes: Recipe[]): Promise<number> {
  const complete = recipes.filter(isComplete);
  const results = await Promise.allSettled(complete.map(saveRecipe));
  return results.filter((r) => r.status === "fulfilled").length;
}
