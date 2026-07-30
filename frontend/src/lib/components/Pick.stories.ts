import type { Meta, StoryObj } from "@storybook/sveltekit";
import Pick from "./Pick.svelte";
import { fullyTimedCard, recipeCards, untimedCard } from "$lib/fixtures";

const meta = {
  title: "recipes/Pick",
  component: Pick,
} satisfies Meta<typeof Pick>;
export default meta;

type Story = StoryObj<typeof meta>;

const cards = recipeCards();
const share = "https://recipes.lehlehleh.com/pick/ab12cd34ef56";

/**
 * The people deciding a plan, for the watching stories (#180).
 *
 * Invented, and it has to be — a plan's roster is who followed a link, and the corpus
 * holds no record of that (`fixtures.ts` says the same about a kitchen's members).
 * The ids are the ones the yes-voter stories already use, so mel is the same colour
 * everywhere in this file, and 9317 carries no username because a Telegram account
 * need not have one and identity is the numeric id.
 */
const deciders = [
  { telegram_user_id: "5150", username: "mel" },
  { telegram_user_id: "3141", username: "kit" },
  { telegram_user_id: "8080", username: "sam" },
];
const nameless = { telegram_user_id: "9317", username: null };

/** A card up to vote on, no consensus yet. The badge beside the category is the
 * recipe's estimated total time (#84): "23 min+", an at-least — this recipe has
 * steps the reading left untimed, and they count as nothing in the total, so the
 * real cook takes longer. */
export const Swiping: Story = {
  args: { status: "swiping", card: cards[0], participants: 2, shareUrl: share },
};

/** The same badge for a recipe whose every step carries a duration (#158): "~19 min",
 * an approximation rather than a floor. Nothing is missing from the total, so it is
 * no longer only-too-low — it is cooking, and the remaining error runs both ways. The
 * mark comes from the card's own `fully_timed`, so as the re-read reaches recipes
 * they cross from the story above to this one, one at a time. */
export const FullyTimedRecipe: Story = {
  args: {
    status: "swiping",
    card: fullyTimedCard(),
    participants: 2,
    shareUrl: share,
  },
};

/** A recipe the step worker has genuinely not read (`total_seconds` is null in the
 * corpus, not blanked for the story): no badge at all. Unknown is not instant —
 * "0 min" would be a lie about the case we know least about — so the card says
 * nothing about time rather than guessing. */
export const UntimedRecipe: Story = {
  args: {
    status: "swiping",
    card: untimedCard(),
    participants: 2,
    shareUrl: share,
  },
};

/** A long one, past the hour: "2 hours 5 min+". Over an hour the badge carries the
 * remainder rather than counting up in minutes, because almost no real recipe lands
 * on a whole hour and "125 min+" is arithmetic, not an answer. */
export const LongRecipe: Story = {
  args: { status: "swiping", card: cards[3], participants: 2, shareUrl: share },
};

/** The card a peer surfaced: it is here *because* Mel voted it, so her yes is named
 * under it in her colour (#131/#145) — the swipe is an answer, not a solo sort. */
export const AlreadyAYesForSomeone: Story = {
  args: {
    status: "swiping",
    card: cards[0],
    participants: 3,
    shareUrl: share,
    yesVoters: [{ telegram_user_id: "5150", username: "mel" }],
  },
};

/**
 * Nearly everyone likes it — four colours under one card, including the two pale
 * slots (`honey` and `sea`). The names carry the meaning and the tints only say
 * whose, which is what lets the pale hues be used at all.
 */
export const SeveralAlreadyYes: Story = {
  args: {
    status: "swiping",
    card: cards[1],
    participants: 5,
    shareUrl: share,
    yesVoters: [
      { telegram_user_id: "5150", username: "mel" },
      { telegram_user_id: "3141", username: "kit" },
      { telegram_user_id: "8080", username: "sam" },
      { telegram_user_id: "9317", username: null },
    ],
  },
};

/** Starting: the socket is opening and the first deck is loading. */
export const Connecting: Story = {
  args: { status: "connecting", shareUrl: share },
};

/** The socket dropped (idle close / spin-down); the banner shows while it re-opens. */
export const Reconnecting: Story = {
  args: {
    status: "reconnecting",
    card: cards[1],
    participants: 3,
    shareUrl: share,
  },
};

/** The deck ran low — a pick is endless, so it's fetching more (never "caught up"). */
export const FindingMore: Story = {
  args: { status: "loading", participants: 3, shareUrl: share },
};

/** Right after copying the invite link. */
export const LinkCopied: Story = {
  args: {
    status: "swiping",
    card: cards[2],
    participants: 2,
    shareUrl: share,
    copied: true,
  },
};

/** The room could not be reached. */
export const Error: Story = {
  args: {
    status: "error",
    error: "Couldn't reach the others (502). Reload the page to rejoin.",
  },
};

/**
 * **Watching** (#180): you opened the link after the swiping began, so you have no
 * seat and no vote — the plan started without you.
 *
 * The card and the tally stay, because that is what watching *is*: seeing what is
 * being decided. What goes is the pair of buttons, and the footer that used to tell a
 * swiper what to do names the people whose decision this is instead. Nothing is
 * greyed out: a disabled Yes still offers itself and still explains nothing, and a
 * vote cast from here would vanish rather than be refused — `record_vote` drops it
 * and the socket that carried it is never answered (#175/#179).
 */
export const Watching: Story = {
  args: {
    status: "swiping",
    card: cards[0],
    participants: 3,
    watching: true,
    roster: deciders,
    shareUrl: share,
  },
};

/**
 * One person is deciding, and it is not you — a plan somebody started alone before
 * sending the link on. The sentence agrees with the roster ("mel **is** deciding"),
 * which is the case a hard-coded plural gets wrong and which the live app reaches the
 * moment a solo host shares a started plan.
 */
export const WatchingOneDecider: Story = {
  args: {
    status: "swiping",
    card: cards[2],
    participants: 1,
    watching: true,
    roster: [deciders[0]],
    shareUrl: share,
  },
};

/**
 * Watching a card two of them already like — the tally a watcher can see, and the
 * reason the deck is not taken away with the buttons. Four deciders, one of them
 * without a username, so the roster line and the yes list both fall back to the
 * numeric id the same way.
 */
export const WatchingWhileTheyAgree: Story = {
  args: {
    status: "swiping",
    card: cards[1],
    participants: 4,
    watching: true,
    roster: [...deciders, nameless],
    yesVoters: [deciders[0], nameless],
    shareUrl: share,
  },
};

/**
 * Watching between cards. A watcher's deck refills like anyone else's, so it runs low
 * like anyone else's — and the line saying whose decision this is rides in the footer
 * precisely so it survives the states that have no card to sit under.
 */
export const WatchingWhileTheDeckRefills: Story = {
  args: {
    status: "loading",
    participants: 3,
    watching: true,
    roster: deciders,
    shareUrl: share,
  },
};
