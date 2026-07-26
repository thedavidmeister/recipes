import { ApiError, apiFetch } from "./client";
import type { Voter } from "./pick";
import { turso } from "./turso";
import type { BuyRecipe, Ingredient, StructuredMeasure } from "./types";

/**
 * `buy` (#36) — the ingredients for the pick's consensus recipe.
 *
 * A pick decides on **one** recipe (consensus); the pick page stashes it here so
 * `buy`, the next step in the meal arc, can list what to get. `localStorage`
 * bridges pick → buy until a durable home exists (a kitchen holds the chosen
 * recipe, #72).
 *
 * The **checklist** no longer lives here (#131). Ticks moved to the meal session,
 * server-side, because a shopping list private to one device was the wrong object:
 * two people shopping the same meal each ticked their own copy and neither ever saw
 * the other's. See {@link getChecks}/{@link setCheck} — and {@link loadChecks} for
 * what is left of the device-local list, and when it is still the right answer.
 */

const KEY = "recipes:consensus";

/** The one recipe a pick decided on. */
export interface ConsensusRef {
  source: string;
  id: string;
  title: string;
  /**
   * The meal session it was decided in — what makes the checklist shared and
   * attributable (#131).
   *
   * Optional, and that is the whole solo/offline story: a decision with no channel
   * has no roster to attribute a tick to, so `buy` falls back to the device-local
   * list. Two real ways to be there — a decision stashed by a build from before
   * this landed, and a browser with no storage at all — and neither should meet a
   * broken page over an attribution nobody asked for.
   */
  channel?: string;
}

/** Record the pick's decision so `buy` can find it. */
export function stashConsensus(ref: ConsensusRef): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(ref));
  } catch {
    // No storage (private mode): buy just shows its empty "pick first" state.
  }
}

/** The recipe the last pick decided on, if any — shared by `buy` and `cook`. */
export function consensusRef(): ConsensusRef | null {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as ConsensusRef) : null;
  } catch {
    return null;
  }
}

/**
 * The ingredients to buy: the consensus recipe's list, read **client-direct** from
 * Turso (the `recipes` table is public). `null` when no pick has decided yet.
 */
export async function getBuyList(): Promise<BuyRecipe | null> {
  const ref = consensusRef();
  if (!ref) return null;

  const rs = await turso().execute({
    sql: "SELECT title, ingredients FROM recipes WHERE source = ? AND id = ? LIMIT 1",
    args: [ref.source, ref.id],
  });
  const row = rs.rows[0];
  const title = row ? String(row.title) : ref.title;

  // The list is the structured reading (#11), never the raw measure — the reading
  // is what `buy` renders. A line with no reading yet is dropped rather than shown
  // raw; `pick` serves read recipes, so a decided one carries readings throughout.
  let ingredients: StructuredMeasure[] = [];
  if (row) {
    try {
      const parsed = JSON.parse(String(row.ingredients)) as Ingredient[];
      ingredients = parsed
        .map((i) => i.structured)
        .filter(
          (s): s is StructuredMeasure => !!s && !!s.item && s.item.trim() !== "",
        );
    } catch {
      // Malformed ingredients JSON: show the recipe with no lines rather than fail.
    }
  }
  return { source: ref.source, id: ref.id, title, ingredients, channel: ref.channel };
}

// ---- the shared checklist (#131) -------------------------------------------

/** One ticked line of the meal's shopping list. Mirrors `session::BuyCheck`. */
export interface BuyCheck {
  /** The 0-based position in the recipe's ingredient list. */
  index: number;
  /** Who has it. */
  by: Voter;
}

/** A meal's checklist for one recipe. Mirrors `session::BuyList`. */
export interface BuyList {
  channel_id: string;
  source: string;
  id: string;
  checks: BuyCheck[];
}

function checksFailed(status: number, action: string): ApiError {
  return new ApiError(
    status,
    status === 401
      ? "Your session has expired."
      : status === 403
        ? "Only the people having this meal can tick things off its list."
        : `could not ${action} (${status})`,
  );
}

/**
 * `GET /api/session/{channel}/buy` — what the group has already got, and who got it.
 *
 * The recipe travels with the request because the checklist is keyed by it: a meal
 * decides on one recipe, but the list is that recipe's list, so a re-decided plan
 * cannot inherit a stale basket.
 */
export async function getChecks(
  channel: string,
  source: string,
  id: string,
): Promise<BuyList> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/buy?source=${encodeURIComponent(source)}&id=${encodeURIComponent(id)}`,
  );
  if (!res.ok) throw checksFailed(res.status, "open the shopping list");
  return (await res.json()) as BuyList;
}

/**
 * `POST /api/session/{channel}/buy` — tick a line off, or put it back.
 *
 * Returns the whole list rather than the one line, because the server does: a tick
 * can take an item off somebody else (last writer wins), so the answer to "what
 * changed" is never confined to the row you touched.
 */
export async function setCheck(
  channel: string,
  source: string,
  id: string,
  index: number,
  checked: boolean,
): Promise<BuyList> {
  const res = await apiFetch(`/api/session/${encodeURIComponent(channel)}/buy`, {
    method: "POST",
    body: JSON.stringify({ source, id, index, checked }),
  });
  if (!res.ok) throw checksFailed(res.status, "update the shopping list");
  return (await res.json()) as BuyList;
}

// ---- the device-local checklist (the solo path) -----------------------------

/**
 * The checklist for a decision with **no session behind it** (see
 * {@link ConsensusRef.channel}).
 *
 * Kept, not deleted. A tick has to land somewhere even when there is nobody to
 * attribute it to, and the alternatives were both worse than a private list: refuse
 * to tick (a shopping list you cannot tick is not a shopping list) or mint a session
 * to hold it (a "meal" with one member, invented by opening a page). So the list
 * degrades to what it always was — this device's, unattributed, unshared — and says
 * so on screen rather than implying a group that is not there.
 */

/** localStorage key for one recipe's shopping checklist (ticked ingredient indices). */
function checksKey(source: string, id: string): string {
  return `recipes:buy-checks:${JSON.stringify([source, id])}`;
}

/** Which ingredient indices are already ticked off for this recipe. */
export function loadChecks(source: string, id: string): number[] {
  try {
    const raw = localStorage.getItem(checksKey(source, id));
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    // localStorage is user-writable, so a non-array (corrupt or legacy) value must
    // not reach the caller — the buy page iterates this result. Keep only numbers.
    return Array.isArray(parsed)
      ? parsed.filter((n): n is number => typeof n === "number")
      : [];
  } catch {
    return [];
  }
}

/** Persist the ticked indices so the checklist survives a reload mid-shop. */
export function saveChecks(source: string, id: string, indices: number[]): void {
  try {
    localStorage.setItem(checksKey(source, id), JSON.stringify(indices));
  } catch {
    // No storage: ticks just won't survive a reload.
  }
}
