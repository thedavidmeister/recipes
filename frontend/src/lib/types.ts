// Mirrors `recipe_core::Recipe` (the shape recipe-wasm returns and the shape
// stored in Turso). Kept in sync with crates/recipe-core/src/models.rs.
export interface Ingredient {
  name: string;
  measure: string | null;
}

/** Render state for a search: what the UI shows, independent of the query lib. */
export type SearchStatus = "idle" | "pending" | "error" | "ready";

/** Mirrors `recipe_core::themealdb::Category`. */
export interface Category {
  name: string;
  thumb: string | null;
  description: string | null;
}

/**
 * The outcome of importing a URL. A discriminated union, not a thrown error:
 * "that page has no recipe" is an ordinary result of pasting a link, and the UI
 * has to distinguish it from a fetch that failed or a save that failed.
 */
export type ImportResult =
  | { kind: "saved"; recipe: Recipe }
  /** A page with schema.org/Recipe data too thin to be worth storing. */
  | { kind: "incomplete"; recipe: Recipe }
  /** Fetched fine, but the page publishes no schema.org/Recipe. */
  | { kind: "no-recipe"; url: string }
  | { kind: "invalid-url"; message: string }
  /** The proxy couldn't fetch it — unreachable, blocked, or non-2xx. */
  | { kind: "fetch-failed"; message: string }
  | { kind: "save-failed"; recipe: Recipe; message: string };

export interface Recipe {
  id: string;
  source: string;
  title: string;
  image: string | null;
  category: string | null;
  area: string | null;
  tags: string[];
  ingredients: Ingredient[];
  instructions: string;
  source_url: string | null;
  video_url: string | null;
}
