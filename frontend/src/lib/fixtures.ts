import type {
  HealthStats,
  Recipe,
  RecipeCard,
  WalkStop,
  Winner,
} from "$lib/types";

// Real TheMealDB records (verified against the live API), shaped the way
// recipe-core normalizes them. Real data keeps stories honest: invented ids and
// image URLs render as unrelated meals.

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
    ingredients: [
      { name: "Chicken", measure: "1.2 kg" },
      { name: "Onion", measure: "5 thinly sliced" },
      { name: "Tomatoes", measure: "2 finely chopped" },
      { name: "Garlic", measure: "8 cloves chopped" },
      { name: "Ginger paste", measure: "1 tbsp" },
      { name: "Vegetable oil", measure: "¼ cup" },
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

/**
 * A pick's candidates with running tallies, for the winners view — over three
 * participants, so the first is both the plurality leader and the consensus pick
 * (everyone said yes), and the rest fall away under consensus.
 */
export function winners(): Winner[] {
  const cards = recipeCards();
  return [
    { card: cards[0], yes: 3, no: 0 },
    { card: cards[1], yes: 3, no: 1 },
    { card: cards[2], yes: 2, no: 1 },
    { card: cards[3], yes: 1, no: 2 },
  ];
}
