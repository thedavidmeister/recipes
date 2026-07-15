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
