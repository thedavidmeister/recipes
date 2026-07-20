import type { Meta, StoryObj } from "@storybook/sveltekit";
import Cook from "./Cook.svelte";
import { cookRecipe } from "$lib/fixtures";

const meta = {
  title: "recipes/Cook",
  component: Cook,
} satisfies Meta<typeof Cook>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The picked recipe in full — image, ingredient reference, and the numbered method. */
export const Ready: Story = {
  args: { status: "ready", recipe: cookRecipe() },
};

/** Loading the recipe. */
export const Pending: Story = {
  args: { status: "pending" },
};

/** No pick has decided yet — nothing to cook. */
export const NoPick: Story = {
  args: { status: "ready", recipe: null },
};

/** The recipe could not be loaded. */
export const Error: Story = {
  args: { status: "error", error: "could not reach the corpus (502)" },
};
