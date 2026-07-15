import {
  ensureWasm,
  normalizeThemealdbSearch,
  normalizeThemealdbCategories,
  normalizeThemealdbMeal,
} from "./wasm";
import type { Category, Recipe } from "./types";

const THEMEALDB = "https://www.themealdb.com/api/json/v1/1";

async function fetchText(url: string, what: string): Promise<string> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`TheMealDB ${what} failed (${res.status})`);
  return res.text();
}

/**
 * Search TheMealDB by name. It sends `Access-Control-Allow-Origin: *`, so the
 * browser fetches it directly; recipe-wasm normalizes the raw JSON.
 */
export async function searchThemealdb(query: string): Promise<Recipe[]> {
  const json = await fetchText(
    `${THEMEALDB}/search.php?s=${encodeURIComponent(query)}`,
    "search",
  );
  await ensureWasm();
  return normalizeThemealdbSearch(json) as Recipe[];
}

/** The category list that drives browsing. */
export async function listCategories(): Promise<Category[]> {
  const json = await fetchText(`${THEMEALDB}/categories.php`, "categories");
  await ensureWasm();
  return normalizeThemealdbCategories(json) as Category[];
}

/**
 * Browse a category. `filter.php` returns header fields only, so these recipes
 * come back **partial** — id, title and image, with no ingredients or
 * instructions. Enough to render a card; not enough to store (see
 * `backend.isComplete`). `category` is filled in from the query, which the
 * response itself omits.
 */
export async function browseCategory(category: string): Promise<Recipe[]> {
  const json = await fetchText(
    `${THEMEALDB}/filter.php?c=${encodeURIComponent(category)}`,
    "category filter",
  );
  await ensureWasm();
  const recipes = normalizeThemealdbSearch(json) as Recipe[];
  return recipes.map((r) => ({ ...r, category: r.category ?? category }));
}

/** Look a meal up by id — the full record behind a browsed partial. */
export async function lookupMeal(id: string): Promise<Recipe | null> {
  const json = await fetchText(
    `${THEMEALDB}/lookup.php?i=${encodeURIComponent(id)}`,
    "lookup",
  );
  await ensureWasm();
  return (normalizeThemealdbMeal(json) as Recipe | null) ?? null;
}
