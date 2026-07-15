import { env } from "$env/dynamic/public";
import { ensureWasm, parseSchemaOrg } from "./wasm";
import type { ImportResult, Recipe } from "./types";

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

/**
 * Fetch a page through the proxy. Browsers can't read arbitrary cross-origin
 * pages, and recipe sites block scrapers, so the backend fetches server-side —
 * SSRF-guarded — and hands back the raw bytes.
 */
async function proxyFetch(url: string): Promise<{ finalUrl: string; body: string }> {
  const res = await fetch(`${backend()}/api/fetch`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ url }),
  });
  if (!res.ok) {
    // The proxy distinguishes a bad request from a blocked target; both are the
    // user's problem to see, not a crash.
    const detail = await res.text().catch(() => "");
    throw new Error(
      `fetch failed (${res.status})${detail ? `: ${detail.slice(0, 200)}` : ""}`,
    );
  }
  const json = await res.json();
  return { finalUrl: json.final_url, body: json.body };
}

/**
 * Import a recipe from any URL: proxy-fetch the page, extract its
 * schema.org/Recipe JSON-LD via recipe-wasm, and save it if it's complete.
 *
 * Returns a discriminated result rather than throwing for the expected cases —
 * "that page has no recipe" is a normal outcome of pasting a URL, not an error,
 * and the UI has to tell them apart.
 */
export async function importFromUrl(url: string): Promise<ImportResult> {
  const trimmed = url.trim();
  if (!trimmed) return { kind: "invalid-url", message: "Enter a URL." };
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return { kind: "invalid-url", message: "Only http(s) URLs can be imported." };
    }
  } catch {
    return { kind: "invalid-url", message: "That doesn't look like a URL." };
  }

  let page: { finalUrl: string; body: string };
  try {
    page = await proxyFetch(trimmed);
  } catch (e) {
    return { kind: "fetch-failed", message: (e as Error).message };
  }

  await ensureWasm();
  // parseSchemaOrg is given the *final* URL so a recipe's source_url reflects
  // where it actually came from after redirects.
  const recipe = parseSchemaOrg(page.body, page.finalUrl) as Recipe | null;
  if (!recipe) return { kind: "no-recipe", url: page.finalUrl };

  if (!isComplete(recipe)) {
    return { kind: "incomplete", recipe };
  }

  try {
    await saveRecipe(recipe);
  } catch (e) {
    return { kind: "save-failed", recipe, message: (e as Error).message };
  }
  return { kind: "saved", recipe };
}
