// Mirrors `recipe_core::Recipe`, the shape the backend returns and the shape
// stored in Turso. Kept in sync with crates/recipe-core/src/models.rs (and
// measure.rs for the structured half).
export interface Ingredient {
  name: string;
  measure: string | null;
  // A model's structured reading of this line (#11), produced by the off-service
  // enrich worker (#59) and reattached at derive. Absent until that worker has
  // read the recipe — the raw name/measure stay the source of truth, so fall back
  // to them when this is absent rather than treating a missing reading as an error.
  structured?: StructuredMeasure | null;
}

// Mirrors `recipe_core::measure::StructuredMeasure`. The enums are internally
// tagged in Rust (`#[serde(tag = "kind")]`), so they arrive as discriminated
// unions on `kind`. The Option<_> fields serialize as `null` when absent
// (matching `measure` above) — only `structured` itself is omitted entirely.
export interface StructuredMeasure {
  item: string;
  amount: Amount | null;
  preparation: string | null;
  note: string | null;
}

export type Amount =
  | {
      kind: "quantified";
      quantity: Quantity;
      unit: string | null;
      size: Size | null;
    }
  | { kind: "qualitative"; text: string };

export interface Size {
  quantity: Quantity;
  unit: string | null;
}

export type Quantity =
  | { kind: "exact"; value: number }
  | { kind: "range"; low: number; high: number };

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
  /** Whether this user is the configured admin — the frontend uses it to offer the
   * health dashboard. Not a security boundary: the admin endpoints re-check. */
  is_admin: boolean;
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

// ---- cook-decider (#20) ----------------------------------------------------

/**
 * Render state of a decider session's swipe view.
 * - `connecting` — opening the room's socket for the first time.
 * - `reconnecting` — the socket dropped (Render's 5-min idle close, or a spin-down);
 *   the client is re-opening and will rehydrate the tally.
 * - `swiping` — a card is up to vote on.
 * - `empty` — nothing left to swipe right now (waiting for peers to surface more).
 * - `error` — the room could not be reached.
 */
export type SessionStatus =
  | "connecting"
  | "reconnecting"
  | "swiping"
  | "empty"
  | "error";

/** The two ways a session decides its winners (#20), selectable in the results. */
export type WinCondition = "plurality" | "consensus";

/** A voted-on recipe with its running tally — the row the winners view ranks. */
export interface Winner {
  card: RecipeCard;
  yes: number;
  no: number;
}

/** One model's enrichment count (`admin::ModelCount`) — provenance at a glance. */
export interface ModelCount {
  model: string;
  count: number;
}

/** A row of the `runs` table (`admin::RunRow`). `finished_at` is null while a run
 * is still going — a long-null one is the died-mid-flight signal. */
export interface RunRow {
  id: number;
  kind: string;
  status: string;
  started_at: number;
  finished_at: number | null;
}

/** The health dashboard's data (`admin::HealthStats`): corpus + enrichment + runs. */
export interface HealthStats {
  recipes: number;
  raw: number;
  enriched: number;
  enriched_pct: number;
  by_model: ModelCount[];
  recent_runs: RunRow[];
  running: number;
}

/**
 * Render state for the admin health dashboard. The page owns the query; the
 * `HealthDashboard` component owns rendering.
 *
 * - `pending` — loading the snapshot.
 * - `error` — the endpoint could not be reached.
 * - `forbidden` — logged in, but not the admin (a 403 from the endpoint).
 * - `ready` — a snapshot is in hand.
 */
export type HealthStatus = "pending" | "error" | "forbidden" | "ready";
