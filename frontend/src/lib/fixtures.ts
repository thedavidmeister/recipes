import sample from "./corpus-sample.json";
import type {
  BuyRecipe,
  CookRecipe,
  HealthStats,
  KitchenDetail,
  KitchenSummary,
  Recipe,
  RecipeCard,
  StructuredMeasure,
  StructuredStep,
  WalkStop,
} from "$lib/types";

/**
 * Story fixtures, built out of **real corpus rows** (#157).
 *
 * CLAUDE.md already required stories to mirror real source records — invented ids
 * and images render the wrong meal. The same applies to every *value* on the row,
 * and hand-typing them failed twice on #84 alone: an `UntimedRecipe` story used a
 * recipe that has a time in production, and every invented round `total_seconds`
 * made an hours-formatting rule look right while it rendered a real 9000s as
 * "150 min+". Invented fixtures don't only misrepresent the corpus; they let code be
 * *fitted to the misrepresentation*.
 *
 * So the corpus half of these fixtures is not typed here at all. It is read out of
 * [`corpus-sample.json`](./corpus-sample.json) — rows dumped verbatim from Turso by
 * `scripts/sample-corpus.mjs`. Refreshing the sample is a deliberate act, like
 * `visual:update`, and the diff is the drift.
 *
 * What is still hand-written is what the *corpus has no record of*: which recipes a
 * story walks, which ingredient a walk crosses, a kitchen's name and members, and an
 * aggregate snapshot that moves every few hours. Each carries a comment naming its
 * source and whether the corpus holds it, and `fixtures.test.ts` pins the ones the
 * corpus can check.
 */

/** A `recipes` row as the sample holds it — `Recipe` plus the columns off it. */
type CorpusRow = Recipe & {
  /** Why this row is in the sample; written by the sampling script. */
  why: string;
  /** `recipes.total_seconds` — the critical path over `steps` (#79/#84). */
  total_seconds: number | null;
  /** `recipes.equipment` — the equipment reading (#81); no surface renders it yet. */
  equipment: { item: string }[];
  /**
   * The nutrition reading's three app-facing columns (#162) — `recipes.kcal`, whether
   * that total is complete, and the servings the card divides it by.
   *
   * Required, like `total_seconds`, because the sample carries them for every row: the
   * badge is rendered from all three at once, and no `??` default stands behind them on
   * purpose. A fallback would let a half-refreshed sample render a whole-recipe total as
   * a per-serving figure while looking perfectly fine on screen; the honest failure for
   * a missing column is the loud one. `fixtures.test.ts` checks at runtime what the cast
   * asserts here.
   */
  kcal: number | null;
  kcal_complete: boolean;
  servings: number | null;
};

/**
 * The sampled rows. One cast, here and nowhere else: the JSON is a verbatim dump of
 * `recipes`, so its shape is the backend's to guarantee, and `fixtures.test.ts`
 * checks at runtime what the cast asserts at compile time.
 */
const CORPUS = sample.recipes as unknown as CorpusRow[];

/** The sampled row for a TheMealDB id, or a loud failure — never a silent `undefined`. */
function row(id: string): CorpusRow {
  const found = CORPUS.find((r) => r.source === "themealdb" && r.id === id);
  if (!found) {
    throw new Error(
      `fixtures: themealdb/${id} is not in corpus-sample.json — add it to WANTED in scripts/sample-corpus.mjs and re-run it`,
    );
  }
  return found;
}

/** The card fields of a sampled row — what the walk and the pick deck render. */
function card(id: string): RecipeCard {
  const r = row(id);
  return {
    source: r.source,
    id: r.id,
    title: r.title,
    image: r.image,
    category: r.category,
    area: r.area,
    total_seconds: r.total_seconds,
    // Measured, not convenient: of the corpus's 790 recipes **not one** carries a
    // duration on every step (2,072 of 9,152 are timed, #158), so every sampled row
    // is a floor and the badge really does read `23 min+`. `fullyTimedCard` is the
    // other state — what these become as the re-read reaches them.
    fully_timed: false,
    // The nutrition reading (#162), straight off the sample — the whole-recipe total,
    // whether it is complete, and the servings the badge divides by. Passed through
    // verbatim and never defaulted: these are the numbers production holds, and a
    // fixture that reshaped any of them would be asserting a calorie count no reading
    // ever made (#157). Measured, and the mirror image of `fully_timed` above: every
    // sampled row's total *is* complete, so the deck's badges read `~701 kcal a
    // serving`. A recipe whose reading left a line unweighable would read
    // `701 kcal+ a serving`, and no sampled row is one — see `Pick.stories.ts`.
    kcal: r.kcal,
    kcal_complete: r.kcal_complete,
    servings: r.servings,
  };
}

/** TheMealDB 52795, Chicken Handi — the recipe `buy` and `cook` are about. */
export const BASE_ID = "52795";

/**
 * The Chicken Handi method as the corpus stores it (#74/#75/#76): `recipes.steps`
 * for TheMealDB 52795, verbatim — five prep roots, a parallel first stage (fry the
 * onion **while** the garlic and tomatoes are chopped), then the long sequential
 * cook. Four steps are timed (60s, 300s, 120s, 900s), and that chain is the whole
 * critical path: 1380s, which is exactly what `recipes.total_seconds` holds.
 *
 * It used to be a hand-authored nine-step DAG whose critical path was 2160s — a
 * number the corpus has never held for this recipe (#157). The cook stories drew
 * their baselines from it, so the whole `cook` surface was a picture of a recipe
 * that does not exist.
 */
export function recipeSteps(): StructuredStep[] {
  return row(BASE_ID).steps;
}

/** TheMealDB 52795 — the base fixture; override fields per story. */
export function recipe(over: Partial<Recipe> = {}): Recipe {
  const r = row(BASE_ID);
  return {
    id: r.id,
    source: r.source,
    title: r.title,
    image: r.image,
    category: r.category,
    area: r.area,
    tags: r.tags,
    // Raw name/measure as the source gave them, each with the enrich worker's
    // structured reading (#11) — what the GUI actually renders. "5 thinly sliced"
    // reads as amount 5 + preparation "thinly sliced": a quantity and a process,
    // never one measure string. All sixteen lines, spelled as the reading spells
    // them (lower-case items): the fixture used to carry eight of them re-cased,
    // plus a "Coriander Leaves / Garnish" line the record does not have.
    ingredients: r.ingredients,
    instructions: r.instructions,
    steps: r.steps,
    source_url: r.source_url,
    video_url: r.video_url,
    ...over,
  };
}

/**
 * TheMealDB 53287 — the **longest title in the corpus** at 81 characters, for the
 * wrapping story. Its own record rather than a long string pasted onto Chicken
 * Handi: a card carrying one meal's title over another's photo is the invented-id
 * failure wearing different clothes.
 */
export function longTitleRecipe(): Recipe {
  const r = row("53287");
  return {
    id: r.id,
    source: r.source,
    title: r.title,
    image: r.image,
    category: r.category,
    area: r.area,
    tags: r.tags,
    ingredients: r.ingredients,
    instructions: r.instructions,
    steps: r.steps,
    source_url: r.source_url,
    video_url: r.video_url,
  };
}

/**
 * The walk as `/api/walk` returns it (#47): sampled corpus rows, threaded by an
 * ingredient each consecutive pair shares. The first stop has no `via` — it is where
 * the wander began. Override for a specific story.
 *
 * Every card field — id, image, category, area, and the `total_seconds` the badge
 * shows (#84) — comes off the sampled row, so none of it can be picked to make a
 * story tidy. The **`via` is the one editorial choice**, because the corpus holds no
 * record of which thread a wander took: it is a hand-picked ingredient, spelled the
 * way the recipe we *left* spells it, and `fixtures.test.ts` pins that it belongs to
 * both adjacent recipes. That is the backend's own invariant
 * (`every_stop_is_reachable_by_its_via` in `walk.rs`), and this fixture used to
 * break it — the Massaman hop claimed "coconut milk", which is in neither the
 * casserole we left (soy sauce, water, brown sugar, ground ginger, …) nor the curry
 * we reached (which has coconut *cream*). A walk the backend is tested never to
 * produce is exactly as false as an invented id.
 *
 * The live app interns one spelling per ingredient corpus-wide and shows the first
 * one it saw, so the case on screen may differ from the case here; membership is the
 * part that is a fact about the record, and membership is what the test holds.
 */
export function walkStops(over: Partial<WalkStop>[] = []): WalkStop[] {
  const base: WalkStop[] = [
    { via: null, recipe: card(BASE_ID) },
    // Chicken Handi spells it "Garam masala"; Katsu Chicken curry has it too.
    { via: "Garam masala", recipe: card("52820") },
    // Both the katsu and the casserole list "soy sauce".
    { via: "soy sauce", recipe: card("52772") },
    // The casserole's "brown sugar" is the Massaman's "Brown sugar".
    { via: "brown sugar", recipe: card("52827") },
    // "Onion" is on both the Massaman and the pie.
    { via: "Onion", recipe: card("52874") },
  ];
  return base.map((stop, i) => ({ ...stop, ...over[i] }));
}

/**
 * The corpus's own health reading, taken **2026-07-28** with
 * `scripts/sample-corpus.mjs` (which prints exactly these numbers alongside the
 * sample it writes).
 *
 * Hand-written rather than derived, deliberately: this is an aggregate, not a
 * record. It moves every few hours as the ingest schedule and the enrich crons run,
 * so pinning it into the committed sample would redraw the health baselines every
 * time an unrelated recipe row was refreshed. The date above is the contract — it
 * says *when* this was true, so a stale number reads as stale instead of as a claim.
 *
 * The reading it replaces (745 recipes / 512 enriched) was real when it was written
 * and had simply drifted. What is worth noticing in today's: **22 runs are still
 * `running`**, two of them the most recent rows. `runs.rs` says a run still running
 * long after `started_at` is one that died mid-flight, and the dashboard's red
 * "in flight" tile is reporting that, not a busy pipeline.
 */
export function healthStats(over: Partial<HealthStats> = {}): HealthStats {
  return {
    recipes: 790,
    raw: 790,
    enriched: 790,
    enriched_pct: 100,
    by_model: [{ model: "claude-sonnet-5", count: 790 }],
    // The five most recent `runs` rows, verbatim. The kind vocabulary is the real
    // one — `enrich_steps` and `enrich_equipment` are pipeline stages the old
    // fixture had never heard of.
    recent_runs: [
      {
        id: 245,
        kind: "ingest",
        status: "running",
        started_at: 1_785_214_894,
        finished_at: null,
      },
      {
        id: 244,
        kind: "ingest",
        status: "running",
        started_at: 1_785_129_309,
        finished_at: null,
      },
      {
        id: 243,
        kind: "derive",
        status: "completed",
        started_at: 1_785_046_724,
        finished_at: 1_785_046_726,
      },
      {
        id: 242,
        kind: "enrich_equipment",
        status: "completed",
        started_at: 1_785_046_721,
        finished_at: 1_785_046_724,
      },
      {
        id: 241,
        kind: "derive",
        status: "completed",
        started_at: 1_785_046_620,
        finished_at: 1_785_046_623,
      },
    ],
    running: 22,
    ...over,
  };
}

/** A deck of real recipe cards for the pick swipe view — the walk's meals. */
export function recipeCards(): RecipeCard[] {
  return walkStops().map((stop) => stop.recipe);
}

/**
 * A card with no time estimate: TheMealDB 53239, whose `total_seconds` is `null` in
 * the live corpus rather than blanked here to make a story (#84).
 *
 * That distinction is the point. Handing a *timed* recipe a `null` would render a
 * meal the app does not have — the same failure as an invented id or image — and it
 * misrepresents the state as rarer or commoner than it is. 77 of the 790 recipes
 * carry no estimate, so the state is real and permanent enough to deserve a real
 * record.
 *
 * Note what the record actually says, because the fixture used to say otherwise: the
 * step worker **has** read this recipe — the sample holds all seven of its steps —
 * and *none of them is timed* ("cook the noodles following pack instructions"). An
 * unread recipe and a read one with no timer both surface as `null`, and this one is
 * the second.
 */
export function untimedCard(): RecipeCard {
  return card("53239");
}

/**
 * A card whose every step carries a duration, so its estimate is an approximation
 * rather than a floor and the badge reads `~19 min` instead of `19 min+` (#158/#84).
 *
 * TheMealDB 53541 — a real record, and the closest thing the live corpus has to this
 * state: 4 steps, of which 3 already carry durations the source stated (300s, 300s,
 * 420s) and one does not — "heat olive oil in a pan over medium-high heat", which
 * takes about 2 minutes whether or not the recipe says so. That is precisely the step
 * the #158 re-read fills in. The chain is linear (0 → 1 → 2 → 3), so the critical
 * path is their sum: the corpus stores 1020 today, counting the unread step as 0, and
 * 1140 once it is read.
 *
 * The 1140 is therefore the one number here the corpus does not yet hold — it cannot,
 * because no recipe in it is fully timed yet. Everything else (id, image, title,
 * category, the three stored durations, the shape of the graph) is the live record,
 * and the arithmetic between them is `total_seconds`'s own.
 */
export function fullyTimedCard(): RecipeCard {
  return {
    source: "themealdb",
    id: "53541",
    title: "Gallo pinto",
    image: "https://www.themealdb.com/images/media/meals/ytogg31784397116.jpg",
    category: "Vegetarian",
    area: null,
    total_seconds: 1140,
    fully_timed: true,
    // Left unread (#162), and it is the only card here that is. This one is
    // hand-written because the state it declares (`fully_timed`) does not exist in the
    // corpus, so 53541 is not in the sample — and the sample is the only place a real
    // calorie figure comes from. Production holds one for this recipe; it is not this
    // file's to guess, and an invented one would assert a count no reading ever made
    // (#157). So this card carries the time badge alone, which is also the honest
    // rendering of the unread nutrition state. Adding 53541 to `WANTED` in
    // `scripts/sample-corpus.mjs` and re-running it is what fills these in.
    kcal: null,
    kcal_complete: false,
    servings: null,
  };
}

/** The structured readings the base fixture carries — what `buy`/`cook` render (#11). */
function readings(): StructuredMeasure[] {
  return recipe()
    .ingredients.map((i) => i.structured)
    .filter((s): s is StructuredMeasure => !!s);
}

/** The consensus recipe's ingredients, for the buy list (the base recipe fixture). */
export function buyRecipe(): BuyRecipe {
  const r = recipe();
  return {
    source: r.source,
    id: r.id,
    title: r.title,
    ingredients: readings(),
  };
}

/**
 * The picked recipe in full, for the cook view — the step DAG to render (#74).
 *
 * Defaults to the base recipe. It takes an id because a method's *shape* is a
 * property of the record: Chicken Handi's stored reading is a single chain, so a
 * story that needs a fork ("at the same time") has to ask a recipe whose reading
 * actually forks rather than invent one that does.
 */
export function cookRecipe(id: string = BASE_ID): CookRecipe {
  const r = row(id);
  return {
    title: r.title,
    image: r.image,
    ingredients: r.ingredients
      .map((i) => i.structured)
      .filter((s): s is StructuredMeasure => !!s),
    steps: r.steps,
  };
}

/**
 * The kitchens a user belongs to (#72), for the kitchens view.
 *
 * Invented, and it has to be: a kitchen is a person's room, written by them — the
 * corpus holds no record of one, so there is nothing real to prefer. Only the
 * *contents* of a kitchen answer to the corpus, and those are handled in
 * [`kitchenDetail`].
 */
export function kitchenList(): KitchenSummary[] {
  return [
    { id: "k1", name: "dave's kitchen", is_primary: true },
    { id: "k2", name: "Beach house", is_primary: false },
    { id: "k3", name: "The Shed", is_primary: false },
  ];
}

/**
 * One kitchen in full — owner + a guest, stocked with equipment and a pantry (#72).
 *
 * The room and its members are invented (see [`kitchenList`]), but what it *holds*
 * is not free to be. Equipment is a picker over what recipes actually ask for (#81),
 * so an item no recipe names could never be added to a real kitchen: every item here
 * is one `equipment_structures` records, with the count of recipes asking for it.
 * "cast-iron pan" was not — no recipe in the corpus asks for one — so a fixture
 * kitchen owned a tool the app would refuse to let anyone own. "wok" replaces it,
 * and is what Chicken Handi itself needs.
 *
 * The pantry comes from the ingredient readings the same way, and every item here
 * already had one.
 */
export function kitchenDetail(): KitchenDetail {
  return {
    id: "k1",
    name: "dave's kitchen",
    is_primary: true,
    members: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: null },
    ],
    // blender 28 · oven 285 · stand mixer 9 · wok 31 (recipes asking for each).
    equipment: ["blender", "oven", "stand mixer", "wok"],
    // basmati rice 10 · eggs 82 · olive oil 153 · soy sauce 53 (readings naming each).
    pantry: ["basmati rice", "eggs", "olive oil", "soy sauce"],
  };
}
