import type { Meta, StoryObj } from "@storybook/sveltekit";
import Buy from "./Buy.svelte";
import { buyRecipe } from "$lib/fixtures";

const meta = {
  title: "recipes/Buy",
  component: Buy,
} satisfies Meta<typeof Buy>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * The six people whose colours the ring can hand out, one per slot. The id is what
 * picks the colour (`userColour` folds its digits), so these are chosen to land one
 * on each: the six-shopper story below only makes its point if every slot — pale
 * ones included — is actually on screen.
 */
const dave = { telegram_user_id: "4242", username: "dave" }; // pesto
const ada = { telegram_user_id: "13579", username: "ada" }; // plum
const mel = { telegram_user_id: "5150", username: "mel" }; // paprika
const kit = { telegram_user_id: "3141", username: "kit" }; // honey
const sam = { telegram_user_id: "8080", username: "sam" }; // sea
const jo = { telegram_user_id: "9317", username: null }; // berry

/** The shopping checklist for the picked recipe — two lines already in a basket,
 * each wearing the colour of whoever got it (#131). */
export const Ready: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: dave, 2: mel },
  },
};

/**
 * A whole household shopping at once: every one of the six colours on one list.
 *
 * This is the story the treatment has to survive. `honey` and `sea` are far too
 * pale to be a control's boundary on cream, so nothing here asks them to be one —
 * the tick is a checked box and a struck-through line, and the colour only says
 * whose. The two pale rows should still read as unmistakably *got*, and still
 * unmistakably somebody's.
 */
export const SixShoppers: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: dave, 1: ada, 2: mel, 3: kit, 4: sam, 5: jo },
  },
};

/**
 * The shop is over (#132): every line of the recipe is ticked, so the list says so
 * and offers the next leg of the arc.
 *
 * A shared meal, so it is the *group's* finish — the six of them between them have
 * everything, and whoever is ready walks on to `cook`. The invitation sits after the
 * list, where the shop ends, and wears `cook`'s paprika dot.
 */
export const Complete: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: dave, 1: ada, 2: mel, 3: kit, 4: sam, 5: jo, 6: ada, 7: dave },
  },
};

/**
 * The same finish with no meal session behind it: the list is this device's, so the
 * completion is this device's too and says exactly that. Nothing here claims a group
 * — the solo path cannot know about one, and pretending otherwise is how somebody
 * walks off to cook while a flatmate is still in the shop.
 */
export const CompleteOnThisDevice: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: false,
    ticks: {
      0: null,
      1: null,
      2: null,
      3: null,
      4: null,
      5: null,
      6: null,
      7: null,
    },
  },
};

/** A tick that did not take: the row goes back to what the server last said, and
 * the reason is on screen. A line that looks got but is not is how somebody comes
 * home without the flour. */
export const TickRefused: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: dave },
    tickError: "Only the people having this meal can tick things off its list.",
  },
};

/** No meal session behind the decision, so there is nobody to attribute a tick to:
 * the list is this device's, unattributed, and says so rather than implying a group
 * that is not there (#131). */
export const OnThisDeviceOnly: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: false,
    ticks: { 0: null, 3: null },
  },
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
