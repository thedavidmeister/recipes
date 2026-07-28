import type { Meta, StoryObj } from "@storybook/sveltekit";
import RecipeCard from "./RecipeCard.svelte";
import { longTitleRecipe, recipe } from "$lib/fixtures";

// `satisfies` (not an annotation): StoryObj<typeof meta> infers args from
// `component`, which only works when typeof meta keeps the literal shape.
const meta = {
  title: "recipes/RecipeCard",
  component: RecipeCard,
} satisfies Meta<typeof RecipeCard>;
export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: { recipe: recipe() },
};

/** Not every source supplies an image — the card must not reserve dead space. A
 * future source, not this one: every row in the corpus today has a photo (0 of 790
 * without), so the `null` here is the card's contract, not a record. */
export const NoImage: Story = {
  args: { recipe: recipe({ image: null }) },
};

/** A recipe with neither category nor area. Half real: 189 of the 790 rows carry no
 * `area`, but none is missing a `category`, so the pair being blank is the card's
 * contract rather than a row you can point at. */
export const NoMeta: Story = {
  args: { recipe: recipe({ category: null, area: null }) },
};

/**
 * Real recipe titles run long; check wrapping. This is the corpus's longest —
 * TheMealDB 53287, 81 characters — as its own record, photo and all.
 *
 * It used to be an invented 96-character title pasted onto Chicken Handi's card: a
 * length no recipe in the corpus reaches, over the wrong meal's photo (#157).
 */
export const LongTitle: Story = {
  args: { recipe: longTitleRecipe() },
};
