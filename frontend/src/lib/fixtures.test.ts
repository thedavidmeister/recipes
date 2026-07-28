import { describe, expect, it } from "vitest";
import sample from "./corpus-sample.json";
import {
  BASE_ID,
  cookRecipe,
  longTitleRecipe,
  recipe,
  recipeSteps,
  untimedCard,
  walkStops,
} from "./fixtures";
import { cookStages, formatEstimate } from "./steps";
import type { Amount, StructuredStep } from "./types";

/**
 * The fixtures answer to the corpus (#157).
 *
 * Story fixtures mirror real source records — including their *values*. `fixtures.ts`
 * reads those out of `corpus-sample.json`, a verbatim dump of `recipes`, so most of
 * that is true by construction. These tests hold the two places it is not:
 *
 * 1. The **cast**. The sample is imported as JSON and asserted into `Recipe` shape at
 *    compile time; nothing checks that assertion at runtime, so this does.
 * 2. The **editorial choices** — which recipe a story picks, and which ingredient a
 *    walk crosses. Those the corpus cannot supply, so they are checked against it.
 *
 * The first assertion is the one this file exists for: `recipeSteps()` claimed a
 * critical path of 2160s while `recipes.total_seconds` for the same recipe is 1380s,
 * and the cook stories drew their baselines from the claim. A fixture whose step
 * graph disagrees with its own estimate is the defect; nothing but arithmetic
 * catches it.
 */

/** The rows as JSON, before the cast — this is what is under test. */
const rows = sample.recipes as {
  source: string;
  id: string;
  title: string;
  image: string | null;
  category: string | null;
  area: string | null;
  tags: unknown;
  ingredients: { name: string; measure: string | null; structured?: unknown }[];
  instructions: string;
  steps: unknown;
  total_seconds: number | null;
}[];

function sampled(id: string) {
  const r = rows.find((row) => row.source === "themealdb" && row.id === id);
  if (!r) throw new Error(`themealdb/${id} missing from the sample`);
  return r;
}

/**
 * The longest chain of timed steps through the DAG — the same critical path
 * `recipes.total_seconds` is derived from (#79). An untimed step contributes
 * nothing but still carries its dependencies, which is what makes the estimate a
 * lower bound rather than a total.
 */
function criticalPath(steps: StructuredStep[]): number {
  const byId = new Map(steps.map((s) => [s.id, s]));
  const memo = new Map<number, number>();
  const of = (id: number): number => {
    const seen = memo.get(id);
    if (seen !== undefined) return seen;
    const step = byId.get(id);
    const before = step?.after.length ? Math.max(...step.after.map(of)) : 0;
    const total = before + (step?.seconds ?? 0);
    memo.set(id, total);
    return total;
  };
  return steps.length ? Math.max(...steps.map((s) => of(s.id))) : 0;
}

describe("the base recipe's step DAG", () => {
  it("has the critical path the corpus stores as its estimate", () => {
    // The defect #157 names, pinned: 1380s, not the 2160s the hand-authored DAG
    // claimed. If the step worker re-reads this recipe, refresh the sample — do not
    // relax this.
    expect(criticalPath(recipeSteps())).toBe(1380);
    expect(sampled(BASE_ID).total_seconds).toBe(criticalPath(recipeSteps()));
  });

  it("is the reading the corpus holds, not a retelling of it", () => {
    expect(recipeSteps()).toEqual(sampled(BASE_ID).steps);
    expect(recipe().ingredients).toEqual(sampled(BASE_ID).ingredients);
  });

  it("estimates as the pick card shows it", () => {
    // 1380s reads as "23 min+" — the at-least the badge promises (#84).
    expect(formatEstimate(sampled(BASE_ID).total_seconds)).toBe("23 min+");
  });
});

describe("the cook stories' recipes", () => {
  /**
   * "At the same time" is a UI state, so a story has to declare it — and whether a
   * method forks is a fact about the recipe, not something a fixture may arrange.
   * The base recipe's stored reading is one chain; Beef and Mustard Pie's forks. If a
   * re-read ever flattens the pie, this fails rather than letting the story quietly
   * stop showing what it claims to.
   */
  const widest = (id: string) =>
    Math.max(...cookStages(cookRecipe(id).steps).map((s) => s.steps.length));

  it("keeps a recipe whose method really forks for the parallel story", () => {
    expect(widest("52874")).toBeGreaterThan(1);
  });

  it("does not claim the base recipe forks", () => {
    expect(widest(BASE_ID)).toBe(1);
  });
});

describe("every sampled row", () => {
  // The runtime half of the cast in `fixtures.ts`: a row whose shape drifted would
  // otherwise reach a story as `undefined` in a template rather than as a failure.
  it("carries the fields a Recipe needs", () => {
    expect(rows.length).toBeGreaterThan(0);
    for (const r of rows) {
      expect(typeof r.source).toBe("string");
      expect(typeof r.id).toBe("string");
      expect(typeof r.title).toBe("string");
      expect(typeof r.instructions).toBe("string");
      expect(Array.isArray(r.tags)).toBe(true);
      expect(Array.isArray(r.ingredients)).toBe(true);
      expect(Array.isArray(r.steps)).toBe(true);
      expect(r.total_seconds === null || typeof r.total_seconds === "number").toBe(
        true,
      );
    }
  });

  it("carries readings and steps in the shapes the types declare", () => {
    for (const r of rows) {
      for (const ing of r.ingredients) {
        expect(typeof ing.name).toBe("string");
        const s = ing.structured as
          | { item: string; amount: Amount | null }
          | null
          | undefined;
        if (!s) continue;
        expect(typeof s.item).toBe("string");
        if (!s.amount) continue;
        expect(["quantified", "qualitative"]).toContain(s.amount.kind);
        if (s.amount.kind === "quantified") {
          expect(["exact", "range"]).toContain(s.amount.quantity.kind);
        }
      }
      for (const step of r.steps as StructuredStep[]) {
        expect(typeof step.id).toBe("number");
        expect(typeof step.text).toBe("string");
        expect(["prep", "cook"]).toContain(step.kind);
        expect(step.seconds === null || typeof step.seconds === "number").toBe(true);
        // `after` only ever references earlier steps — the backend validates it, and
        // `stepDepths` recurses on it.
        for (const dep of step.after) expect(dep).toBeLessThan(step.id);
      }
    }
  });
});

describe("the walk fixture", () => {
  const stops = walkStops();

  it("begins with no thread and threads every later stop", () => {
    expect(stops[0].via).toBeNull();
    for (const stop of stops.slice(1)) expect(stop.via).toBeTruthy();
  });

  /**
   * The backend's own invariant (`every_stop_is_reachable_by_its_via` in
   * `walk.rs`): the ingredient crossed belongs to the recipe left *and* the one
   * reached. The fixture used to cross "coconut milk" between a teriyaki casserole
   * and a Massaman curry, and neither has any.
   */
  it("crosses an ingredient both recipes actually hold", () => {
    for (let i = 1; i < stops.length; i++) {
      const via = stops[i].via!.toLowerCase();
      const from = sampled(stops[i - 1].recipe.id);
      const to = sampled(stops[i].recipe.id);
      const names = (r: typeof from) =>
        r.ingredients.map((ing) => ing.name.trim().toLowerCase());
      expect(names(from), `${from.title} -> ${to.title}`).toContain(via);
      expect(names(to), `${from.title} -> ${to.title}`).toContain(via);
    }
  });

  it("spells the thread the way the recipe it left spells it", () => {
    for (let i = 1; i < stops.length; i++) {
      const from = sampled(stops[i - 1].recipe.id);
      expect(from.ingredients.map((ing) => ing.name)).toContain(stops[i].via);
    }
  });

  it("shows each recipe's own stored estimate", () => {
    for (const stop of stops) {
      expect(stop.recipe.total_seconds).toBe(sampled(stop.recipe.id).total_seconds);
    }
  });
});

describe("the cards a story picks for a state", () => {
  it("uses a recipe the corpus really has no estimate for", () => {
    // Not a timed recipe blanked to make a story — the failure #84 shipped.
    expect(untimedCard().total_seconds).toBeNull();
    expect(sampled(untimedCard().id).total_seconds).toBeNull();
    expect(formatEstimate(untimedCard().total_seconds)).toBeNull();
  });

  it("uses the longest title in the sample for the wrapping story", () => {
    // The sample's longest is the *corpus's* longest (81 chars, TheMealDB 53287 —
    // see WANTED in `scripts/sample-corpus.mjs`). Offline, the sample is as far as a
    // test can check; what it does check is that the story is not quietly pointed at
    // a shorter row.
    const longest = rows.reduce((a, b) =>
      b.title.length > a.title.length ? b : a,
    );
    expect(longTitleRecipe().title).toBe(longest.title);
    // Its own record, photo included — not one meal's title over another's image.
    expect(longTitleRecipe().image).toBe(longest.image);
  });
});
