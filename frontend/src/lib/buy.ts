import { turso } from "./turso";
import type { BuyRecipe, Ingredient, StructuredMeasure } from "./types";

/**
 * `buy` (#36) — the ingredients for the pick's consensus recipe.
 *
 * A pick decides on **one** recipe (consensus); the pick page stashes it here so
 * `buy`, the next step in the meal arc, can list what to get. `localStorage`
 * bridges pick → buy until a durable home exists (a kitchen holds the chosen
 * recipe, #72).
 */

const KEY = "recipes:consensus";

/** The one recipe a pick decided on. */
export interface ConsensusRef {
  source: string;
  id: string;
  title: string;
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
  return { source: ref.source, id: ref.id, title, ingredients };
}

/** localStorage key for one recipe's shopping checklist (ticked ingredient indices). */
function checksKey(source: string, id: string): string {
  return `recipes:buy-checks:${JSON.stringify([source, id])}`;
}

/** Which ingredient indices are already ticked off for this recipe. */
export function loadChecks(source: string, id: string): number[] {
  try {
    const raw = localStorage.getItem(checksKey(source, id));
    return raw ? (JSON.parse(raw) as number[]) : [];
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
