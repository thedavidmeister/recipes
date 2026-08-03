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

/**
 * The people cooking a plan, for the shared-timer stories (#208).
 *
 * Invented, and it has to be — a plan's roster is who followed a link, and the corpus
 * holds no record of that. The ids are `Pick.stories.ts`'s, so mel and kit are the same
 * colours everywhere they appear, and 9317 carries no username because a Telegram
 * account need not have one and identity is the numeric id.
 */
const mel = { telegram_user_id: "5150", username: "mel" };
const kit = { telegram_user_id: "3141", username: "kit" };
const nameless = { telegram_user_id: "9317", username: null };

/**
 * **The plan's timers, not this phone's** (#208) — the same two steps as `Timers`
 * above, on a cook with a meal session behind it, so each countdown says whose pot it
 * is: mel started the garlic (step 6, now done) and kit the tomatoes (step 7, 4:12
 * left).
 *
 * The point of the picture is that these are the *same* numbers on everybody's screen.
 * The deadline behind them is one instant on a shared timeline — mel's tap, corrected
 * for mel's own clock drift — so this render is what mel, kit and anybody else in the
 * plan are all looking at, rather than three private countdowns that happen to be near
 * each other.
 */
export const SharedTimers: Story = {
  args: {
    status: "ready",
    recipe: cookRecipe(),
    timers: {
      6: { remaining: 0, done: true, by: mel },
      7: { remaining: 252, done: false, by: kit },
    },
  },
};

/**
 * A shared timer that has run out, started by somebody with no Telegram username — so
 * the attribution falls back to the numeric id, which is the identity anyway.
 *
 * Its own story rather than a corner of the one above, because "done" is the state a
 * cook acts on: the pot comes off the heat, and whoever is nearest dismisses it for the
 * whole room at once.
 */
export const SharedDone: Story = {
  args: {
    status: "ready",
    recipe: cookRecipe(),
    timers: {
      7: { remaining: 0, done: true, by: nameless },
    },
  },
};

/**
 * **Watching a cook** (#180/#200): the plan's countdowns are all here and not one
 * control is. Step 6 is done, step 7 is counting down, and every other timed step shows
 * how long it takes as a plain duration rather than as a Start button.
 *
 * Reachable because the decision goes to the *room*: somebody who joined a plan after
 * the swiping began never got a seat, and is carried through to this screen with
 * everybody else. There is no disabled button anywhere in this render — the server
 * refuses a watcher's start in silence, so a control that looked pressable would be a
 * control that did nothing without saying why.
 */
export const Watching: Story = {
  args: {
    status: "ready",
    recipe: cookRecipe(),
    watching: true,
    timers: {
      6: { remaining: 0, done: true, by: mel },
      7: { remaining: 252, done: false, by: kit },
    },
  },
};

/**
 * A method that forks: Beef and Mustard Pie (TheMealDB 52874), whose stored reading
 * has two parallel stages — one of them three steps wide — so "At the same time"
 * has a record behind it.
 *
 * Its own recipe rather than a fork spliced into Chicken Handi's graph, because
 * whether a method forks is a fact about the recipe. Chicken Handi's real reading is
 * a single chain from the first sauté to the cream; the nine-step fixture this
 * replaces gave it a parallel stage it does not have, and the `Ready` story above
 * was the picture of that (#157).
 */
export const Parallel: Story = {
  args: { status: "ready", recipe: cookRecipe("52874") },
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
  args: {
    status: "error",
    error: "The server didn't answer (502). Try again in a moment.",
  },
};
