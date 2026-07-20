// Mirrors `recipe_core::Recipe`, the shape the backend returns and the shape
// stored in Turso. Kept in sync with crates/recipe-core/src/models.rs (and
// measure.rs for the structured half).
export interface Ingredient {
  name: string;
  measure: string | null;
  // A model's structured reading of this line (#11), produced by the off-service
  // enrich worker (#59) and reattached at derive. This reading is what the GUI
  // renders — the raw name/measure are the worker's input, not a display form, so
  // the app never falls back to them. Absent until the worker has read the recipe;
  // enrichment is an addition to the corpus, not a gate on it.
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

/** Whether a step is mise en place or active cooking (#76). Mirrors `StepKind`. */
export type StepKind = "prep" | "cook";

/**
 * One node in a recipe's method DAG (#74/#75/#76), the model's reading of the
 * instructions. Mirrors `recipe_core::StructuredStep`. `id` is 0-based; `after`
 * holds the ids of the steps that must finish first (`[]` = can start now), so
 * parallel-vs-sequential is derived from the edges. `seconds` is a timer's duration
 * when the step is timed.
 */
export interface StructuredStep {
  id: number;
  text: string;
  kind: StepKind;
  seconds: number | null;
  after: number[];
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
  /** The model's reading of `instructions` into a step DAG (#74/#75/#76), attached
   * at derive. Empty until the step-reading worker has read the recipe. */
  steps: StructuredStep[];
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

// ---- pick (#20) ------------------------------------------------------------

/**
 * Render state of a pick's swipe view.
 * - `connecting` — starting the pick: opening the socket + loading the first deck.
 * - `reconnecting` — the socket dropped (Render's 5-min idle close, or a spin-down);
 *   the client is re-opening and will rehydrate the tally.
 * - `swiping` — a card is up to vote on.
 * - `loading` — the deck ran low; fetching more from the walk. A pick is **endless**
 *   until you decide (there is no "caught up"), so this is a brief bridge, not a stop.
 * - `error` — the room could not be reached.
 */
export type PickStatus =
  | "connecting"
  | "reconnecting"
  | "swiping"
  | "loading"
  | "error";

/**
 * A **match** (#20): a recipe everyone in the pick said yes to. Consensus is the
 * whole point — a match is the pick, surfaced inline the moment it happens.
 */
export interface Match {
  card: RecipeCard;
  /** How many said yes — equals the participant count for a match. */
  yes: number;
}

// ---- buy (#36) -------------------------------------------------------------

/** Render state of the buy list. */
export type BuyStatus = "pending" | "error" | "ready";

/**
 * A consensus recipe from a pick, with the ingredients to buy for it. `buy` is
 * the arc after `pick` (#36): what the group agreed on, and what it needs.
 * `source`/`id` key the persisted checklist (what's already in the basket).
 *
 * Ingredients are the structured reading (#11) — `item` + measured `amount`, never
 * the raw measure. `buy` shows what to get and how much; preparation ("finely
 * diced") is a `cook` concern, not a shopping one.
 */
export interface BuyRecipe {
  source: string;
  id: string;
  title: string;
  ingredients: StructuredMeasure[];
}

// ---- cook (#36) ------------------------------------------------------------

/** Render state of the cook view. */
export type CookStatus = "pending" | "error" | "ready";

/**
 * The picked recipe in full, for cooking (#36) — the step after `buy`. The method
 * is the star; the ingredients ride along as a reference.
 *
 * Both halves are the structured reading, never raw: ingredients are the measure
 * reading (#11), and `steps` is the method read into a DAG (#74/#75/#76) — a timer
 * per timed step, and the dependencies that say what runs in parallel.
 */
export interface CookRecipe {
  title: string;
  image: string | null;
  ingredients: StructuredMeasure[];
  steps: StructuredStep[];
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
