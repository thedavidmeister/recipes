import type { Recipe } from "$lib/types";

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
