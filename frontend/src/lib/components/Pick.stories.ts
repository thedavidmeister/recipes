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

/**
 * The same empty deck, for the other reason (#202): this person has answered every
 * recipe the plan can currently deal them, so there is nothing to find. Waiting on the
 * others is a real state — a plan runs for days and people come and go — and the one
 * thing it must never be is the story above, hunting forever for a card that is not
 * coming.
 */
export const AnsweredEverything: Story = {
  args: { status: "loading", finished: true, participants: 3, shareUrl: share },
};

/** Two in the plan, so one other is left to hear from — said in the singular. */
export const AnsweredEverythingWaitingOnOne: Story = {
  args: { status: "loading", finished: true, participants: 2, shareUrl: share },
};

/**
 * Nobody else is in this plan. The roster closed at the start (#96/#169), so no one is
 * coming, and "0 others are still deciding" would be the same holding pattern dressed
 * up as a number.
 */
export const AnsweredEverythingAlone: Story = {
  args: { status: "loading", finished: true, participants: 1, shareUrl: share },
};

/**
 * Finished *and* reconnecting. The two are independent — one is a fact about the deal,
 * the other about the socket — which is why finishing is its own prop rather than
 * another status: as a status it would lose this race, and the person who has answered
 * everything would be told a card is on its way.
 */
export const AnsweredEverythingWhileReconnecting: Story = {
  args: {
    status: "reconnecting",
    finished: true,
    participants: 3,
    shareUrl: share,
  },
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
