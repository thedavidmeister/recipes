import type {
  Amount,
  BuyRecipe,
  CookRecipe,
  HealthStats,
  Recipe,
  RecipeCard,
  StructuredMeasure,
  WalkStop,
} from "$lib/types";

// Real TheMealDB records (verified against the live API), shaped the way
// recipe-core normalizes them. Real data keeps stories honest: invented ids and
// image URLs render as unrelated meals.

/** An exact quantity with an optional unit — the common `Amount` in a reading (#11). */
function exact(value: number, unit: string | null = null): Amount {
  return { kind: "quantified", quantity: { kind: "exact", value }, unit, size: null };
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
    source_url: null,
    video_url: "https://www.youtube.com/watch?v=IO0issT0Rmc",
    ...over,
  };
}

/**
 * A walk, as `/api/walk` returns it: real TheMealDB meals (ids/images verified
 * against the live corpus), threaded by an ingredient each pair shares. The first
 * stop has no `via` — it is where the wander began. Override for a specific story.
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

/** The picked recipe in full, for the cook view — multi-step method to render. */
export function cookRecipe(): CookRecipe {
  const r = recipe();
  return {
    title: r.title,
    image: r.image,
    ingredients: readings(),
    instructions: [
      "Heat the oil in a large pot and add the sliced onions.",
      "Once golden, stir in the ginger paste and garlic and fry for a minute.",
      "Add the tomatoes and cook until they break down into a sauce.",
      "Add the chicken and brown it on all sides.",
      "Pour in a cup of water, cover, and simmer for 30 minutes.",
      "Finish with fresh coriander and serve.",
    ].join("\n"),
  };
}
