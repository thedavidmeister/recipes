import type { Meta, StoryObj } from "@storybook/sveltekit";
import Cook from "./Cook.svelte";
import { cookRecipe } from "$lib/fixtures";

const meta = {
  title: "recipes/Cook",
  component: Cook,
} satisfies Meta<typeof Cook>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * The picked recipe in full — its sixteen ingredients, the prep lane, and the method
 * as stages. This is Chicken Handi's stored reading (#157), not a nine-step
 * retelling of it: sixteen steps, four of them timed, and a critical path of 1380s
 * — the number the corpus holds as this recipe's estimate.
 */
export const Ready: Story = {
  args: { status: "ready", recipe: cookRecipe() },
};

/**
 * Timers in flight, on the recipe's real timed steps: the one-minute garlic sauté
 * (step 6) has fired, and the five-minute tomato cook it leads to (step 7) is
 * counting down with 4:12 left.
 *
 * The pair is in dependency order deliberately — 7 comes `after` 6 — so it is a
 * moment a cook can actually be in. The invented DAG this replaces had its two
 * timers running a step and its own prerequisite at the same time.
 */
export const Timers: Story = {
  args: {
    status: "ready",
    recipe: cookRecipe(),
    timers: {
      6: { remaining: 0, done: true },
      7: { remaining: 252, done: false },
    },
  },
};

/** Loading the recipe. */
export const Pending: Story = {
  args: { status: "pending" },
};

/** The recipe's method hasn't been read into steps yet — a state every recipe passes
 * through between the ingest that adds it and the step worker's next run. No record
 * is in it right now (all 790 have a reading), so this one is blanked on purpose. */
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
