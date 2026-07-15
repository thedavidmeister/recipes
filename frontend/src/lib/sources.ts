import { ensureWasm, normalizeDocument, normalizeThemealdbCategories } from "./wasm";
import type { Category, Recipe } from "./types";

const THEMEALDB = "https://www.themealdb.com/api/json/v1/1";

async function fetchText(url: string, what: string): Promise<string> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`TheMealDB ${what} failed (${res.status})`);
  return res.text();
}

/**
 * Fetch a document from a source and normalize it through the adapter registry.
 *
 * Everything that enters the corpus goes through here: recipe-core routes the
 * document to the adapter claiming the host, and **throws for a host no adapter
 * claims**. The corpus is a cache of sources we support, not arbitrary pages, so
 * an unknown source fails closed instead of being parsed best-effort.
 */
async function ingest(url: string, what: string): Promise<Recipe[]> {
  const body = await fetchText(url, what);
  await ensureWasm();
  // The host is parsed here: recipe-core deliberately has no URL parser, to keep
  // `url`/`idna` out of the wasm bundle.
  return normalizeDocument(new URL(url).hostname, url, body) as Recipe[];
}

/**
 * Search TheMealDB by name. It sends `Access-Control-Allow-Origin: *`, so the
 * browser fetches it directly.
 */
export async function searchThemealdb(query: string): Promise<Recipe[]> {
  return ingest(`${THEMEALDB}/search.php?s=${encodeURIComponent(query)}`, "search");
}

/** The category list that drives browsing. Categories are a taxonomy, not recipes. */
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
  const recipes = await ingest(
    `${THEMEALDB}/filter.php?c=${encodeURIComponent(category)}`,
    "category filter",
  );
  return recipes.map((r) => ({ ...r, category: r.category ?? category }));
}

/** Look a meal up by id — the full record behind a browsed partial. */
export async function lookupMeal(id: string): Promise<Recipe | null> {
  const recipes = await ingest(
    `${THEMEALDB}/lookup.php?i=${encodeURIComponent(id)}`,
    "lookup",
  );
  return recipes[0] ?? null;
}
