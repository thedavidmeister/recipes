import type { Meta, StoryObj } from "@storybook/sveltekit";
import Buy from "./Buy.svelte";
import { NOBODY, type Tick } from "$lib/shopping";
import { buyRecipe } from "$lib/fixtures";
import type { Voter } from "$lib/pick";

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

/** Somebody got it: the row wears their colour and says their name (#131). */
const got = (by: Voter): Tick => ({ by, pantry: null });

/**
 * The kitchen already had it (#156): nobody got it, so no colour — the entry that
 * answered for the line goes where a name would.
 *
 * The names used are real corpus vocabulary with real weight behind them — `salt`
 * is in 302 of the 790 recipes, `garlic` 238, `onion` 224, `vegetable oil` 116. The
 * fixture is Chicken Handi, and the indices below are positions in *its own*
 * sixteen-line record (#157), so a pantry holding `tomato` pre-ticks `tomatoes` —
 * one food, two spellings, which is the whole of what the matcher's plural fold
 * does. Positions are the thing to get right here: `salt` is the sixteenth line,
 * and while these ticks were written against an earlier eight-line fixture the
 * salt sat on the seventh, which pre-ticked `cumin seeds` — a fixture asserting
 * exactly the false match this matcher exists to refuse.
 */
const inPantry = (item: string): Tick => ({ by: null, pantry: item });

/** The shopping checklist for the picked recipe — two lines already got,
 * each wearing the colour of whoever got it (#131). */
export const Ready: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: got(dave), 2: got(mel) },
  },
};

/**
 * The list as it is **born** when the plan's kitchen is stocked (#156): the staples
 * are already accounted for and nobody had to do anything.
 *
 * This is the treatment on its own, with no people on the list at all — plain stone,
 * no colour anywhere, and each ticked row naming the jar. Read it against `Ready`
 * above: a tick that somebody made is coloured, a tick nobody made is not, and that
 * difference is the only thing carrying the distinction.
 */
export const StartedFromThePantry: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: {
      1: inPantry("onion"),
      2: inPantry("tomato"),
      3: inPantry("garlic"),
      5: inPantry("vegetable oil"),
      15: inPantry("salt"),
    },
  },
};

/**
 * The story the two treatments have to survive together: a real shop in progress on
 * a list that started stocked.
 *
 * Three rows are somebody's and wear their colour; three are the kitchen's and wear
 * none. Nothing about a pantry row may read as a person having claimed it — that is
 * #131's rule (a colour means a person) held honest against a tick that has no
 * person behind it — and nothing about a person's row may read as a cupboard.
 *
 * `Vegetable oil` at index 5 is the interesting one: it was pre-ticked from the
 * pantry, the jar turned out to be empty, and `@kit` went and got some. Taking one
 * over is an ordinary tick, so it looks like an ordinary tick.
 */
export const PantryAndPeople: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: {
      0: got(dave),
      2: inPantry("tomato"),
      3: inPantry("garlic"),
      5: got(kit),
      15: inPantry("salt"),
      7: got(jo),
    },
  },
};

/**
 * Nothing to buy at all: every line of the recipe was already in the kitchen, so the
 * list is finished the moment it opens and nobody shopped for any of it.
 *
 * #132's completion said *"Everything's in the kitchen"* — true here, but it sits
 * under a heading that would be congratulating a group on a shop none of them did.
 * So the words change and the arithmetic does not. Measured against a plausible
 * 30-item staple pantry this is rare and real: 2 of the corpus's 790 recipes are
 * born complete.
 */
export const NothingToBuy: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: {
      0: inPantry("chicken"),
      1: inPantry("onion"),
      2: inPantry("tomato"),
      3: inPantry("garlic"),
      4: inPantry("ginger paste"),
      5: inPantry("vegetable oil"),
      6: inPantry("cumin seeds"),
      7: inPantry("coriander seeds"),
      8: inPantry("turmeric powder"),
      9: inPantry("chilli powder"),
      10: inPantry("green chilli"),
      11: inPantry("yogurt"),
      12: inPantry("cream"),
      13: inPantry("fenugreek"),
      14: inPantry("garam masala"),
      15: inPantry("salt"),
    },
  },
};

/**
 * The same full list, but one line was actually shopped for — so it *is* a finished
 * shop and says so the way #132 always did. One person's tick anywhere is the whole
 * threshold, and this pins that the two endings do not blur into each other.
 */
export const CompleteAfterOneRealShop: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: {
      0: got(mel),
      1: inPantry("onion"),
      2: inPantry("tomato"),
      3: inPantry("garlic"),
      4: inPantry("ginger paste"),
      5: inPantry("vegetable oil"),
      6: inPantry("cumin seeds"),
      7: inPantry("coriander seeds"),
      8: inPantry("turmeric powder"),
      9: inPantry("chilli powder"),
      10: inPantry("green chilli"),
      11: inPantry("yogurt"),
      12: inPantry("cream"),
      13: inPantry("fenugreek"),
      14: inPantry("garam masala"),
      15: inPantry("salt"),
    },
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
    ticks: {
      0: got(dave),
      1: got(ada),
      2: got(mel),
      3: got(kit),
      4: got(sam),
      5: got(jo),
    },
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
    // All sixteen of the recipe's lines — the corpus record has sixteen, not the
    // eight the fixture used to carry (#157), and "every line" has to mean every one.
    ticks: {
      0: got(dave),
      1: got(ada),
      2: got(mel),
      3: got(kit),
      4: got(sam),
      5: got(jo),
      6: got(ada),
      7: got(dave),
      8: got(mel),
      9: got(kit),
      10: got(sam),
      11: got(jo),
      12: got(dave),
      13: got(ada),
      14: got(mel),
      15: got(kit),
    },
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
      0: NOBODY,
      1: NOBODY,
      2: NOBODY,
      3: NOBODY,
      4: NOBODY,
      5: NOBODY,
      6: NOBODY,
      7: NOBODY,
      8: NOBODY,
      9: NOBODY,
      10: NOBODY,
      11: NOBODY,
      12: NOBODY,
      13: NOBODY,
      14: NOBODY,
      15: NOBODY,
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
    ticks: { 0: got(dave) },
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
    ticks: { 0: NOBODY, 3: NOBODY },
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

/** The picked recipe has no ingredients listed. Constructed, not sampled: no row in
 * the corpus has an empty ingredient list (checked — 0 of 790), so there is no real
 * record to point this at, and the source/id here key nothing real either. */
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
