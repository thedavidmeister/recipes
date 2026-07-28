import type {
  Amount,
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

// Real TheMealDB records (verified against the live API), shaped the way
// recipe-core normalizes them. Real data keeps stories honest: invented ids and
// image URLs render as unrelated meals.

/** An exact quantity with an optional unit — the common `Amount` in a reading (#11). */
function exact(value: number, unit: string | null = null): Amount {
  return { kind: "quantified", quantity: { kind: "exact", value }, unit, size: null };
}

/**
 * The Chicken Handi method read into a step DAG (#74/#75/#76): three prep roots, a
 * parallel cook stage (fry the onions **while** blending the tomatoes), then the
 * sequential finish. Three steps are timed (fry 5:00, bloom 1:00, simmer 30:00).
 */
export function recipeSteps(): StructuredStep[] {
  return [
    { id: 0, text: "Thinly slice the onions", kind: "prep", seconds: null, after: [] },
    { id: 1, text: "Chop the garlic and ginger", kind: "prep", seconds: null, after: [] },
    { id: 2, text: "Finely chop the tomatoes", kind: "prep", seconds: null, after: [] },
    { id: 3, text: "Fry the onions until golden", kind: "cook", seconds: 300, after: [0] },
    { id: 4, text: "Meanwhile, blend the tomatoes into a purée", kind: "cook", seconds: null, after: [2] },
    { id: 5, text: "Stir the garlic, ginger, and tomato purée into the onions", kind: "cook", seconds: 60, after: [3, 4, 1] },
    { id: 6, text: "Add the chicken and brown it all over", kind: "cook", seconds: null, after: [5] },
    { id: 7, text: "Pour in a cup of water, cover, and simmer", kind: "cook", seconds: 1800, after: [6] },
    { id: 8, text: "Finish with fresh coriander and serve", kind: "cook", seconds: null, after: [7] },
  ];
}

/** TheMealDB 52795 — the base fixture; override fields per story. */
export function recipe(over: Partial<Recipe> = {}): Recipe {
  return {
    id: "52795",
    source: "themealdb",
    title: "Chicken Handi",
    image: "https://www.themealdb.com/images/media/meals/wyxwsp1486979827.jpg",
    category: "Chicken",
    area: "India",
    tags: [],
    // Raw name/measure as the source gave them, each with the enrich worker's
    // structured reading (#11) — what the GUI actually renders. "5 thinly sliced"
    // reads as amount 5 + preparation "thinly sliced": a quantity and a process,
    // never one measure string.
    ingredients: [
      {
        name: "Chicken",
        measure: "1.2 kg",
        structured: { item: "Chicken", amount: exact(1.2, "kg"), preparation: null, note: null },
      },
      {
        name: "Onion",
        measure: "5 thinly sliced",
        structured: { item: "Onion", amount: exact(5), preparation: "thinly sliced", note: null },
      },
      {
        name: "Tomatoes",
        measure: "2 finely chopped",
        structured: { item: "Tomatoes", amount: exact(2), preparation: "finely chopped", note: null },
      },
      {
        name: "Garlic",
        measure: "8 cloves chopped",
        structured: { item: "Garlic", amount: exact(8, "cloves"), preparation: "chopped", note: null },
      },
      {
        name: "Ginger paste",
        measure: "1 tbsp",
        structured: { item: "Ginger paste", amount: exact(1, "tbsp"), preparation: null, note: null },
      },
      {
        name: "Vegetable oil",
        measure: "¼ cup",
        structured: { item: "Vegetable oil", amount: exact(0.25, "cup"), preparation: null, note: null },
      },
      {
        name: "Salt",
        measure: "To taste",
        structured: {
          item: "Salt",
          amount: { kind: "qualitative", text: "to taste" },
          preparation: null,
          note: null,
        },
      },
      {
        name: "Coriander Leaves",
        measure: "Garnish",
        structured: { item: "Coriander Leaves", amount: null, preparation: null, note: "to garnish" },
      },
    ],
    instructions:
      "Take a large pot or wok, big enough to cook all the chicken, and heat the oil in it. Once the oil is hot, add sliced onions.",
    steps: recipeSteps(),
    source_url: null,
    video_url: "https://www.youtube.com/watch?v=IO0issT0Rmc",
    ...over,
  };
}

/**
 * A walk, as `/api/walk` returns it: real TheMealDB meals (ids/images verified
 * against the live corpus), threaded by an ingredient each pair shares. The first
 * stop has no `via` — it is where the wander began. Override for a specific story.
 *
 * `total_seconds` is the estimate the card shows (#84), and each one is the value
 * the live corpus actually holds for that recipe — read out of Turso, not picked to
 * make a story tidy. A number that merely *looks* right is the same mistake as an
 * invented id or image: it renders a recipe that does not exist. Every meal on this
 * walk has been read by the step worker; for the ~10% of the corpus that has not,
 * see [`untimedCard`].
 *
 * `fully_timed` is `false` on every one of them, and that is measured rather than
 * convenient: of 790 recipes in the corpus, **not one** currently carries a duration
 * on every step (2,072 of 9,152 steps are timed). So each of these totals really is
 * only a floor, and the badge really does read `23 min+`. [`fullyTimedCard`] is the
 * other state — what these become as the #158 re-read reaches them.
 */
export function walkStops(over: Partial<WalkStop>[] = []): WalkStop[] {
  const base: WalkStop[] = [
    {
      via: null,
      recipe: {
        source: "themealdb",
        id: "52795",
        title: "Chicken Handi",
        image:
          "https://www.themealdb.com/images/media/meals/wyxwsp1486979827.jpg",
        category: "Chicken",
        area: "India",
        total_seconds: 1380,
        fully_timed: false,
      },
    },
    {
      via: "garam masala",
      recipe: {
        source: "themealdb",
        id: "52820",
        title: "Katsu Chicken curry",
        image:
          "https://www.themealdb.com/images/media/meals/vwrpps1503068729.jpg",
        category: "Chicken",
        area: "Japanese",
        total_seconds: 1980,
        fully_timed: false,
      },
    },
    {
      via: "soy sauce",
      recipe: {
        source: "themealdb",
        id: "52772",
        title: "Teriyaki Chicken Casserole",
        image:
          "https://www.themealdb.com/images/media/meals/wvpsxx1468256321.jpg",
        category: "Chicken",
        area: "Japanese",
        total_seconds: 3360,
        fully_timed: false,
      },
    },
    {
      via: "coconut milk",
      recipe: {
        source: "themealdb",
        id: "52827",
        title: "Massaman Beef curry",
        image:
          "https://www.themealdb.com/images/media/meals/tvttqv1504640475.jpg",
        category: "Beef",
        area: "Thai",
        total_seconds: 7500,
        fully_timed: false,
      },
    },
    {
      via: "onion",
      recipe: {
        source: "themealdb",
        id: "52874",
        title: "Beef and Mustard Pie",
        image:
          "https://www.themealdb.com/images/media/meals/sytuqu1511553755.jpg",
        category: "Beef",
        area: "British",
        total_seconds: 9000,
        fully_timed: false,
      },
    },
  ];
  return base.map((stop, i) => ({ ...stop, ...over[i] }));
}

/**
 * A realistic mid-enrichment snapshot — the real corpus size (745), part-read.
 * Fixed unix timestamps so the runs table renders identically in every capture.
 * Override per story (empty corpus, a stuck run, etc.).
 */
export function healthStats(over: Partial<HealthStats> = {}): HealthStats {
  return {
    recipes: 745,
    raw: 745,
    enriched: 512,
    enriched_pct: (512 / 745) * 100,
    by_model: [{ model: "claude-sonnet-5", count: 512 }],
    recent_runs: [
      { id: 27, kind: "enrich", status: "completed", started_at: 1_752_849_600, finished_at: 1_752_849_642 },
      { id: 26, kind: "derive", status: "completed", started_at: 1_752_849_598, finished_at: 1_752_849_600 },
      { id: 25, kind: "ingest", status: "completed", started_at: 1_752_846_000, finished_at: 1_752_846_071 },
      { id: 24, kind: "enrich", status: "failed", started_at: 1_752_838_800, finished_at: 1_752_838_815 },
      { id: 23, kind: "ingest", status: "completed", started_at: 1_752_760_800, finished_at: 1_752_760_863 },
    ],
    running: 0,
    ...over,
  };
}

/** A deck of real recipe cards for the pick swipe view — the walk's meals. */
export function recipeCards(): RecipeCard[] {
  return walkStops().map((stop) => stop.recipe);
}

/**
 * A card the step worker has genuinely not read: `total_seconds` is `null` in the
 * live corpus, not blanked here to make a story (#84).
 *
 * That distinction is the point. Handing a *timed* recipe a `null` would render a
 * meal the app does not have — the same failure as an invented id or image, and it
 * misrepresents the state as rarer or commoner than it is. Roughly a tenth of the
 * corpus is unread at any time (77 of 790 when this was written), so the state is
 * real and permanent enough to deserve a real record: TheMealDB 53239.
 */
export function untimedCard(): RecipeCard {
  return {
    source: "themealdb",
    id: "53239",
    title: "Bang bang prawn salad",
    image: "https://www.themealdb.com/images/media/meals/4xcfai1763765676.jpg",
    category: "Seafood",
    area: "Vietnamese",
    total_seconds: null,
    fully_timed: false,
  };
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
  return { source: r.source, id: r.id, title: r.title, ingredients: readings() };
}

/** The picked recipe in full, for the cook view — the step DAG to render (#74). */
export function cookRecipe(): CookRecipe {
  const r = recipe();
  return {
    title: r.title,
    image: r.image,
    ingredients: readings(),
    steps: recipeSteps(),
  };
}

/** The kitchens a user belongs to (#72), for the kitchens view. */
export function kitchenList(): KitchenSummary[] {
  return [
    { id: "k1", name: "dave's kitchen", is_primary: true },
    { id: "k2", name: "Beach house", is_primary: false },
    { id: "k3", name: "The Shed", is_primary: false },
  ];
}

/** One kitchen in full — owner + a guest, stocked with equipment and a pantry (#72). */
export function kitchenDetail(): KitchenDetail {
  return {
    id: "k1",
    name: "dave's kitchen",
    is_primary: true,
    members: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: null },
    ],
    equipment: ["blender", "cast-iron pan", "oven", "stand mixer"],
    pantry: ["basmati rice", "eggs", "olive oil", "soy sauce"],
  };
}
