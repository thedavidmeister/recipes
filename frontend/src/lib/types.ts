// Mirrors `recipe_core::Recipe`, the shape the backend returns and the shape
// stored in Turso. Kept in sync with crates/recipe-core/src/models.rs.
export interface Ingredient {
  name: string;
  measure: string | null;
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

/** Mirrors `auth::MeResponse`. Identity is the Telegram id; the username is a
 * display name and may be absent — a Telegram account need not have one. */
export interface User {
  telegram_user_id: string;
  username: string | null;
}

/**
 * Render state for the login screen. Auth is mandatory (#25), so this is the
 * first thing a visitor meets.
 *
 * - `checking` — asking `/api/me` whether a session already exists.
 * - `idle` — no session; point at the bot.
 * - `error` — the backend could not be reached.
 */
export type LoginStatus = "checking" | "idle" | "error";

/** The four top-level destinations (#36): the arc of a meal. */
export type Section = "pick" | "buy" | "cook" | "joy";

/**
 * A recipe as the walk returns it — the read fields, no ingredients or
 * instructions. Mirrors `backend::walk::RecipeCard`: a card, not the full page.
 */
export interface RecipeCard {
  source: string;
  id: string;
  title: string;
  image: string | null;
  category: string | null;
  area: string | null;
}

/**
 * One stop on a walk (`backend::walk::Stop`). `via` is the ingredient crossed to
 * reach this recipe — the thread that makes a walk read as a journey; it is `null`
 * only for the first stop, which was arrived at by nothing.
 */
export interface WalkStop {
  via: string | null;
  recipe: RecipeCard;
}

/**
 * Render state for the `pick` walk. The page owns the query; the `Walk` component
 * owns rendering, and takes this so every state is a Storybook story rather than
 * something you race the network to see.
 *
 * - `pending` — the first walk is loading.
 * - `error` — the walk could not be fetched.
 * - `ready` — a walk is in hand (possibly empty, if the corpus is).
 */
export type WalkStatus = "pending" | "error" | "ready";
