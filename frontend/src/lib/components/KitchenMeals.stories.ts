import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenMeals from "./KitchenMeals.svelte";
import { decidedRecipe, kitchenMeals } from "$lib/fixtures";

const meta = {
  title: "recipes/KitchenMeals",
  component: KitchenMeals,
} satisfies Meta<typeof KitchenMeals>;
export default meta;

type Story = StoryObj<typeof meta>;

const meals = kitchenMeals();

/**
 * A kitchen with all three states in it at once, newest first — which is what a room
 * that cooks together actually looks like: a dinner being gathered for, a lunch already
 * being swiped, and last night's dinner, settled.
 *
 * The recipe named at the bottom is a real corpus record (#157/#205); the plans and
 * their rosters are invented, because a plan is who followed a link and the corpus holds
 * no record of one.
 */
export const AllThreeStates: Story = {
  args: { status: "ready", meals },
};

/**
 * The lobby, on its own: people are still arriving, so the count is what is in *so far*
 * and it is going up. Tapping the row seats you (#96) — arrival is joining.
 */
export const Gathering: Story = {
  args: { status: "ready", meals: [meals[0]] },
};

/**
 * The swiping has begun. The roster closed at the start in both directions (#96), so
 * this count is final — it is the number a recipe has to win over, not a tally of who
 * has turned up.
 */
export const Deciding: Story = {
  args: { status: "ready", meals: [meals[1]] },
};

/**
 * Decided (#205): the row names the dish. The outcome is a server fact, recorded inside
 * the deciding vote's own write, so this is read rather than worked out from a tally —
 * which is what lets a meal that finished days ago still say what it was.
 */
export const Decided: Story = {
  args: { status: "ready", meals: [meals[2]] },
};

/**
 * One person in each — the singular copy, which is the ordinary case for a kitchen of
 * one and the state a plan is in for its first minute (a plan seats its host as it is
 * made, so a lobby is never empty).
 */
export const JustOnePerson: Story = {
  args: {
    status: "ready",
    meals: kitchenMeals([{ deciders: 1 }, { deciders: 1 }]).slice(0, 2),
  },
};

/**
 * The longest title in the corpus (81 characters) as an outcome, so the row is declared
 * at the width it has to survive. Its own record rather than a long string pasted onto
 * another meal — a card carrying one dish's name over another's plan is the invented-id
 * failure wearing different clothes (#157).
 */
export const DecidedALongTitle: Story = {
  args: {
    status: "ready",
    meals: kitchenMeals([{}, {}, { decided: decidedRecipe("53287") }]).slice(2),
  },
};

/**
 * A kitchen nobody has planned a meal in. Said quietly and honestly — there is nothing
 * to pad it with — and it points at the "Let's cook!" button already on the page rather
 * than offering a second way to start one.
 */
export const NoMealsYet: Story = {
  args: { status: "ready", meals: [] },
};

export const Pending: Story = { args: { status: "pending" } };

export const Failed: Story = {
  args: {
    status: "error",
    error: "You've been signed out. Sign in again to carry on.",
  },
};
