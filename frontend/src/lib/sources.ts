import { env } from "$env/dynamic/public";
import type { Category, Recipe } from "./types";

const THEMEALDB = "https://www.themealdb.com/api/json/v1/1";

function backend(): string {
  const url = env.PUBLIC_BACKEND_URL;
  if (!url) throw new Error("PUBLIC_BACKEND_URL is not set");
  return url.replace(/\/$/, "");
}

/**
 * Ask the server to ingest a document: fetch it, derive recipes, store both
 * halves, and hand back what it found.
 *
 * The client **drives** ingestion (it decides what to look for); the server
 * **performs** it. That is why nothing is parsed here — the browser's copy of
 * the normalizer only existed to parse arbitrary pages it had fetched itself,
 * and the corpus no longer ingests arbitrary pages. It also lets a source need
 * a credential, which a public SPA could never hold.
 *
 * The server fails closed on a host no adapter claims.
 */
async function ingest(url: string, what: string): Promise<Recipe[]> {
  const res = await fetch(`${backend()}/api/ingest`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ url }),
  });
  if (!res.ok) throw new Error(`${what} failed (${res.status})`);
  const json = await res.json();
  return json.recipes as Recipe[];
}

/** Search TheMealDB by name. */
export async function searchThemealdb(query: string): Promise<Recipe[]> {
  return ingest(`${THEMEALDB}/search.php?s=${encodeURIComponent(query)}`, "search");
}

/**
 * Browse a category. `filter.php` returns header fields only, so these recipes
 * come back **partial** — enough to render a card, and deliberately not stored
 * (the server only stores complete ones). `category` is filled in from the
 * query, which the response itself omits.
 */
export async function browseCategory(category: string): Promise<Recipe[]> {
  const found = await ingest(
    `${THEMEALDB}/filter.php?c=${encodeURIComponent(category)}`,
    "category browse",
  );
  return found.map((r) => ({ ...r, category: r.category ?? category }));
}

/** Look a meal up by id — the full record behind a browsed partial. */
export async function lookupMeal(id: string): Promise<Recipe | null> {
  const found = await ingest(`${THEMEALDB}/lookup.php?i=${encodeURIComponent(id)}`, "lookup");
  return found[0] ?? null;
}

/**
 * The category list that drives browsing. A taxonomy, not recipes — nothing to
 * ingest, so it stays a direct read (TheMealDB sends permissive CORS).
 */
export async function listCategories(): Promise<Category[]> {
  const res = await fetch(`${THEMEALDB}/categories.php`);
  if (!res.ok) throw new Error(`categories failed (${res.status})`);
  const json = await res.json();
  return (json.categories ?? []).map(
    (c: Record<string, string>): Category => ({
      name: c.strCategory,
      thumb: c.strCategoryThumb ?? null,
      description: c.strCategoryDescription ?? null,
    }),
  );
}
