import type { Meta, StoryObj } from "@storybook/sveltekit";
import RecipeCard from "./RecipeCard.svelte";
import { recipe } from "$lib/fixtures";

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

/** Not every source supplies an image — the card must not reserve dead space. */
export const NoImage: Story = {
  args: { recipe: recipe({ image: null }) },
};

/** Imported (non-TheMealDB) recipes often lack category/area entirely. */
export const NoMeta: Story = {
  args: { recipe: recipe({ category: null, area: null }) },
};

/** Real schema.org titles run long; check wrapping. */
export const LongTitle: Story = {
  args: {
    recipe: recipe({
      title:
        "Slow-Roasted Harissa Chicken Thighs with Preserved Lemon, Green Olives and Herbed Couscous",
    }),
  },
};
