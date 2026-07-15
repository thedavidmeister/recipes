// Mirrors `recipe_core::Recipe`, the shape the backend returns and the shape
// stored in Turso. Kept in sync with crates/recipe-core/src/models.rs.
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

/** Mirrors `auth::MeResponse`. Identity is the Telegram id; the username is a
 * display name and may be absent — a Telegram account need not have one. */
export interface User {
  telegram_user_id: string;
  username: string | null;
}

/** What `POST /api/auth/start` hands back. */
export interface LoginStart {
  /** `t.me/<bot>?start=<nonce>` — shareable by design. */
  link: string;
  /** Redeems the session. Must never go in the link. */
  pollSecret: string;
  expiresAt: number;
}

/** Mirrors `auth::PollResponse`. No token: on `ready` the session arrived as an
 * HttpOnly cookie the browser holds and script cannot read. */
export type PollResult =
  | { status: "pending" }
  | { status: "ready"; username: string | null }
  | { status: "expired" };

/**
 * Render state for the login screen. Auth is mandatory (#25), so this is the
 * first thing a visitor meets.
 *
 * - `checking` — asking `/api/me` whether a session already exists.
 * - `idle` — no session; offer to start.
 * - `starting` — minting a login.
 * - `waiting` — link is up; waiting for the tap.
 * - `expired` — nobody tapped in time.
 * - `error` — the backend could not be reached or refused.
 */
export type LoginStatus =
  | "checking"
  | "idle"
  | "starting"
  | "waiting"
  | "expired"
  | "error";
