import type { Meta, StoryObj } from "@storybook/sveltekit";
import Buy from "./Buy.svelte";
import { buyRecipe } from "$lib/fixtures";

const meta = {
  title: "recipes/Buy",
  component: Buy,
} satisfies Meta<typeof Buy>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The shopping checklist for the picked recipe — a couple already ticked off. */
export const Ready: Story = {
  args: { status: "ready", recipe: buyRecipe(), checked: { 0: true, 2: true } },
};

/** Loading the list. */
export const Pending: Story = {
  args: { status: "pending" },
};

/** No pick has decided yet — nothing to buy. */
export const NoPick: Story = {
  args: { status: "ready", recipe: null },
};

/** The picked recipe has no ingredients listed. */
export const NoIngredients: Story = {
  args: {
    status: "ready",
    recipe: { source: "themealdb", id: "1", title: "Toast", ingredients: [] },
  },
};

/** The list could not be loaded. */
export const Error: Story = {
  args: { status: "error", error: "could not reach the corpus (502)" },
};
