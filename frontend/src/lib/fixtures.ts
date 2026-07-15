import type { Category, Recipe } from "$lib/types";

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

/** Real TheMealDB categories (categories.php), trimmed to a browsable few. */
export const categories: Category[] = [
  {
    name: "Beef",
    thumb: "https://www.themealdb.com/images/category/beef.png",
    description:
      "Beef is the culinary name for meat from cattle, particularly skeletal muscle.",
  },
  {
    name: "Chicken",
    thumb: "https://www.themealdb.com/images/category/chicken.png",
    description:
      "Chicken is a type of domesticated fowl, a subspecies of the red junglefowl.",
  },
  {
    name: "Dessert",
    thumb: "https://www.themealdb.com/images/category/dessert.png",
    description: "Dessert is a course that concludes a meal.",
  },
  {
    name: "Lamb",
    thumb: "https://www.themealdb.com/images/category/lamb.png",
    description:
      "Lamb, hogget, and mutton are the meat of domestic sheep (species Ovis aries).",
  },
  {
    name: "Miscellaneous",
    thumb: "https://www.themealdb.com/images/category/miscellaneous.png",
    description: "General foods that don't fit into another category",
  },
  {
    name: "Seafood",
    thumb: "https://www.themealdb.com/images/category/seafood.png",
    description: "Seafood is any form of sea life regarded as food by humans.",
  },
];

/**
 * A category browse result — `filter.php` returns header fields only, so these
 * are deliberately partial: no ingredients, no instructions.
 */
export const partialRecipes: Recipe[] = [
  {
    id: "52874",
    source: "themealdb",
    title: "Beef and Mustard Pie",
    image: "https://www.themealdb.com/images/media/meals/sytuqu1511553755.jpg",
    category: "Beef",
    area: null,
    tags: [],
    ingredients: [],
    instructions: "",
    source_url: null,
    video_url: null,
  },
  {
    id: "52878",
    source: "themealdb",
    title: "Beef and Oyster pie",
    image: "https://www.themealdb.com/images/media/meals/wrssvt1511556563.jpg",
    category: "Beef",
    area: null,
    tags: [],
    ingredients: [],
    instructions: "",
    source_url: null,
    video_url: null,
  },
];

export const recipes: Recipe[] = [
  recipe(),
  recipe({
    id: "53358",
    title: "Chicken Mandi",
    area: "India",
    image: "https://www.themealdb.com/images/media/meals/er4d081765186828.jpg",
    video_url: null,
  }),
  recipe({
    id: "53110",
    title: "Sticky Chicken",
    area: "Australian",
    image: "https://www.themealdb.com/images/media/meals/cj56fs1762340001.jpg",
    source_url: "https://www.bbcgoodfood.com/recipes/sticky-chicken",
    video_url: "https://www.youtube.com/watch?v=o8tz2BOltTg",
  }),
];
