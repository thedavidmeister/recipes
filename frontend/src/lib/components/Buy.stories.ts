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
 * **A tap of your own, before the room has answered for it** (#210).
 *
 * `dave` is the person at this screen. `vegetable oil` is the tap he has just made and
 * the server has not announced yet; `chicken` is `mel`'s, `tomatoes` is the pantry's,
 * and `garlic` is a tick of `dave`'s the room confirmed a minute ago. The point of the
 * story is that you cannot tell his two rows apart — an in-flight tick is the tapper's,
 * colour and name, from the first paint, so when the room's `buy` frame lands it
 * **confirms** the row rather than repainting it.
 *
 * Read against `Ready`: this is deliberately indistinguishable from a list where every
 * tick is announced, and that indistinguishability *is* the fix. What it replaced was a
 * row that painted plain stone and then flipped to the tapper's colour a round trip
 * later — a colour flash on every single tick, exactly at the moment of interaction.
 */
export const YourTapBeforeTheRoomAnswers: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: got(mel), 2: inPantry("tomato"), 3: got(dave), 5: got(dave) },
  },
};

/**
 * **The same list, that tap refused** — where the screen settles afterwards (#210).
 *
 * Exactly `YourTapBeforeTheRoomAnswers` with one row changed, so the pair reads as the
 * before and after of one refusal. A tick the server would not take still gets a
 * whole-list frame back, and that frame is the truth: `vegetable oil` was never got, so
 * it returns to ordinary paper. What this pins is what is *not* on it — no tint, no
 * accent, no name left behind on a line that carries no tick. The colour was never a
 * second piece of state that could survive the tick; it was part of the tick, and it
 * went back with it.
 *
 * `dave`'s `garlic` is a tick that *was* taken, so the rollback is visibly local to the
 * refused line rather than a wipe of everything he has.
 */
export const YourTapRolledBack: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    ticks: { 0: got(mel), 2: inPantry("tomato"), 3: got(dave) },
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
 * everything, and whoever taps "Let's cook!" takes the whole room to the stove with them
 * (#211). The invitation sits after the list, where the shop ends, and wears `cook`'s
 * paprika dot.
 *
 * On a shared list the control is a **button**, not a link, and the difference is the
 * point rather than a detail: following an `href` is one person going to `/cook`, which
 * is exactly the bug — starting the cook is a thing that happens to the meal, so it is
 * raised on the plan's room and every screen moves when the announcement comes back.
 * `CompleteOnThisDevice` below is the same finish with nobody to bring, and keeps the
 * plain link it always had.
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
 *
 * "Let's cook!" is therefore the plain link it has always been (#211): there is no room
 * to raise an event on and nobody to bring along, so nothing about this path changed.
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

/**
 * **Watching somebody else's shop** (#200/#222) — the state `buy` never had.
 *
 * Somebody who opened a plan that had already started is on no roster, so the server
 * refuses every tick they could make. This is what they get instead: the whole list,
 * every attribution on it — `mel`'s two lines, `kit`'s one, the jar that answered for
 * the tomatoes — and no box to tap. The tick is *stated* where it used to be *offered*:
 * a solid square in the colour of whoever got it, an outline where nobody has it.
 *
 * Read it against `PantryAndPeople` above, which is the same list for somebody who is
 * shopping it. Everything that says *what is got and whose* is identical; the only
 * difference is that nothing here is pressable, and the footer says whose shop it is —
 * `Pick`'s watching line, one page along, because watching means one thing everywhere.
 *
 * It is the story that pins the suppression: dropping `watching` from `Buy` restores
 * sixteen tappable checkboxes and takes the footer away, and the visual fence sees both.
 */
export const Watching: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    watching: true,
    roster: [mel, kit, sam],
    ticks: { 0: got(mel), 2: inPantry("tomato"), 5: got(kit), 8: got(mel) },
  },
};

/**
 * A watcher on a **finished** list: the shop is over, the room is about to cook, and
 * there is still nothing here for them to press.
 *
 * "Let's cook!" goes with the checkboxes, and for the same reason — `Guard::
 * SeatedInDecidedPlan` refuses a watcher's `cook_started`, so a button would offer
 * itself and explain nothing (`Pick`'s rule about a greyed-out **Yes**, one page along).
 * They are not left behind by it: the room's `cooking` frame reaches every socket, so a
 * watcher is carried to the stove by the announcement exactly as they were carried here
 * by the decision.
 *
 * Read against `Complete`, which is the same finish for somebody who is shopping it.
 */
export const WatchingAFinishedList: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    watching: true,
    roster: [mel, kit],
    ticks: {
      0: got(mel),
      1: got(kit),
      2: got(mel),
      3: got(kit),
      4: got(mel),
      5: got(kit),
      6: got(mel),
      7: got(kit),
      8: got(mel),
      9: got(kit),
      10: got(mel),
      11: got(kit),
      12: got(mel),
      13: got(kit),
      14: got(mel),
      15: got(kit),
    },
  },
};

/**
 * Watching before the roster has been read.
 *
 * The lobby is a second request, so there is a frame in which this screen knows it is
 * watching and cannot yet name anybody. The sentence holds without the names rather
 * than blinking a half-built list of them into place — the plan is somebody else's
 * either way, which is the part that explains the missing boxes.
 */
export const WatchingBeforeTheRosterArrives: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: true,
    watching: true,
    roster: [],
    ticks: { 0: got(mel), 2: inPantry("tomato") },
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

/** No meal session behind the decision, and this browser's stored list is all there
 * is: these ticks were made in some earlier sitting, by whoever was at this browser
 * then, so nobody can be named for them and none of them wears a colour (#131). */
export const OnThisDeviceOnly: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: false,
    ticks: { 0: NOBODY, 3: NOBODY },
  },
};

/**
 * The same private list a moment later: `dave` has ticked line 5 **here and now**, so
 * that one is his (#210) while the two restored from storage stay nobody's.
 *
 * Both halves are the same rule read in both directions. There is no meal session, but
 * "no session" was never "no person" — the tap happened at this screen and this screen
 * knows who is at it, so the row is his from the first paint exactly as a shared one
 * would be. Lines 0 and 3 came out of `localStorage`, which outlives a sitting and is
 * shared with anyone else who uses this browser, so they have no author to claim and
 * are not given one. The list still says *just on this device*: a colour on it names
 * who got a thing, never who else can see it.
 */
export const OnThisDeviceWithYourOwnTick: Story = {
  args: {
    status: "ready",
    recipe: buyRecipe(),
    shared: false,
    ticks: { 0: NOBODY, 3: NOBODY, 5: got(dave) },
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
  args: {
    status: "error",
    error: "The server didn't answer (502). Try again in a moment.",
  },
};

