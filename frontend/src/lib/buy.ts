import { turso } from "./turso";
import type { BuyRecipe } from "./types";

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

function consensusRef(): ConsensusRef | null {
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

  let ingredients: BuyRecipe["ingredients"] = [];
  if (row) {
    try {
      const parsed = JSON.parse(String(row.ingredients)) as {
        name: string;
        measure: string | null;
      }[];
      ingredients = parsed
        .filter((i) => i.name && i.name.trim() !== "")
        .map((i) => ({ name: i.name, measure: i.measure ?? null }));
    } catch {
      // Malformed ingredients JSON: show the recipe with no lines rather than fail.
    }
  }
  return { title, ingredients };
}
