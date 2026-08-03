import { describe, expect, it } from "vitest";
import sample from "./corpus-sample.json";
import {
  BASE_ID,
  cookRecipe,
  kitchenMeals,
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
  // The nutrition reading (#162), carried by every sampled row.
  kcal: number | null;
  kcal_complete: boolean;
  servings: number | null;
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
      expect(
        r.total_seconds === null || typeof r.total_seconds === "number",
      ).toBe(true);
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
        expect(step.seconds === null || typeof step.seconds === "number").toBe(
          true,
        );
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
      expect(stop.recipe.total_seconds).toBe(
        sampled(stop.recipe.id).total_seconds,
      );
    }
  });

  /**
   * The calorie badge (#162) may only ever say what the sample says. This is the #157
   * rule pointed at the newest column, and it is the one a story is most tempted to
   * break: a calorie figure is a plausible-looking number nobody reading the story can
   * check, so a fixture that invented one would let the badge's display rules be fitted
   * to a recipe that does not exist.
   *
   * It holds in both directions of the refresh. Today the sample carries no reading, so
   * this pins that no card deals itself one; once `sample-corpus.mjs` has been re-run
   * against production, it pins that every card carries the corpus's own numbers rather
   * than the last hand-typed ones.
   */
  it("deals no calorie figure the corpus sample does not hold", () => {
    for (const stop of stops) {
      const r = sampled(stop.recipe.id);
      expect(stop.recipe.kcal).toBe(r.kcal);
      expect(stop.recipe.servings).toBe(r.servings);
      expect(stop.recipe.kcal_complete).toBe(r.kcal_complete);
    }
  });

  /**
   * A total and the servings it is divided by are written together by
   * `recipes::upsert`, so a sampled row carrying one without the other is a half-read
   * row — and the badge would silently render a whole-recipe total as a per-serving
   * figure if it ever tried. Vacuous while the sample predates the columns; a real
   * check the moment it is refreshed, which is exactly when it is needed.
   */
  it("never carries a total without the servings that interpret it", () => {
    for (const r of rows) {
      if (typeof r.kcal !== "number") continue;
      expect(typeof r.servings, `${r.title} has kcal but no servings`).toBe(
        "number",
      );
      expect(r.servings).toBeGreaterThan(0);
    }
  });

  /**
   * The runtime half of the now-required nutrition columns on `CorpusRow` — the same
   * job the `total_seconds` check above does. A sample re-dumped by a script that had
   * quietly lost a column from its `SELECT` would otherwise reach a story as
   * `undefined` and render as an unread recipe: a badge that vanished corpus-wide and
   * looked exactly like an honest absence (the #161 failure, in fixture clothing).
   */
  it("carries all three nutrition columns on every row", () => {
    for (const r of rows) {
      expect(r.kcal === null || typeof r.kcal === "number").toBe(true);
      expect(r.servings === null || typeof r.servings === "number").toBe(true);
      expect(typeof r.kcal_complete, `${r.title} kcal_complete`).toBe(
        "boolean",
      );
    }
  });
});

describe("the kitchen's meals fixture", () => {
  const meals = kitchenMeals();

  /**
   * The half a kitchen listing cannot invent (#157/#205). A plan is a room of people
   * who followed a link and the corpus holds no record of one, so the channels and
   * rosters here are written by hand — but a decided meal states what a group is
   * having, and that is a corpus record. A made-up title would be the invented-id
   * failure with the recipe's own name on it, and nobody reading the story could tell.
   */
  it("names only recipes the corpus sample holds", () => {
    for (const meal of meals) {
      if (!meal.decided) continue;
      const r = sampled(meal.decided.id);
      expect(meal.decided.source).toBe(r.source);
      expect(meal.decided.title).toBe(r.title);
    }
  });

  /**
   * The list declares all three states a plan can be in, which is what the kitchen
   * page's headline story is claiming to show. A fixture that quietly lost one would
   * leave that state visible in no story at all.
   */
  it("holds one meal in each of the three states", () => {
    expect(meals.filter((m) => !m.started && !m.decided)).toHaveLength(1);
    expect(meals.filter((m) => m.started && !m.decided)).toHaveLength(1);
    expect(meals.filter((m) => m.decided)).toHaveLength(1);
  });

  /**
   * A decided meal has necessarily been swiped, so `started` is not free to be false
   * beside a decision: that pair is a plan that decided without ever dealing a card,
   * which the server cannot produce (a vote is refused until the lobby closes) and a
   * fixture must not draw.
   */
  it("never shows a decision on a plan that never started", () => {
    for (const meal of meals) {
      if (meal.decided) expect(meal.started).toBe(true);
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
