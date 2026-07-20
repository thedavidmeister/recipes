import type { Meta, StoryObj } from "@storybook/sveltekit";
import Cook from "./Cook.svelte";
import { cookRecipe } from "$lib/fixtures";

const meta = {
  title: "recipes/Cook",
  component: Cook,
} satisfies Meta<typeof Cook>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The picked recipe in full — ingredients, the prep lane, and the method as stages
 * (one stage runs two steps in parallel). Timed steps offer a Start control. */
export const Ready: Story = {
  args: { status: "ready", recipe: cookRecipe() },
};

/** Timers in flight: the fry step counting down, the simmer step finished. */
export const Timers: Story = {
  args: {
    status: "ready",
    recipe: cookRecipe(),
    timers: {
      3: { remaining: 252, done: false },
      7: { remaining: 0, done: true },
    },
  },
};

/** Loading the recipe. */
export const Pending: Story = {
  args: { status: "pending" },
};

/** The recipe's method hasn't been read into steps yet. */
export const Unread: Story = {
  args: { status: "ready", recipe: { ...cookRecipe(), steps: [] } },
};

/** No pick has decided yet — nothing to cook. */
export const NoPick: Story = {
  args: { status: "ready", recipe: null },
};

/** The recipe could not be loaded. */
export const Error: Story = {
  args: { status: "error", error: "could not reach the corpus (502)" },
};
