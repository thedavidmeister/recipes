import { ensureWasm, normalizeThemealdbSearch } from "./wasm";
import type { Recipe } from "./types";

const THEMEALDB = "https://www.themealdb.com/api/json/v1/1";

/**
 * Search TheMealDB by name. It sends `Access-Control-Allow-Origin: *`, so the
 * browser fetches it directly; recipe-wasm normalizes the raw JSON.
 */
export async function searchThemealdb(query: string): Promise<Recipe[]> {
  const res = await fetch(
    `${THEMEALDB}/search.php?s=${encodeURIComponent(query)}`,
  );
  if (!res.ok) throw new Error(`TheMealDB search failed (${res.status})`);
  const json = await res.text();
  await ensureWasm();
  return normalizeThemealdbSearch(json) as Recipe[];
}
