import type { Meta, StoryObj } from "@storybook/sveltekit";
import Pick from "./Pick.svelte";
import { recipeCards, untimedCard } from "$lib/fixtures";

const meta = {
  title: "recipes/Pick",
  component: Pick,
} satisfies Meta<typeof Pick>;
export default meta;

type Story = StoryObj<typeof meta>;

const cards = recipeCards();
const share = "https://recipes.lehlehleh.com/pick/ab12cd34ef56";

/** A card up to vote on, no consensus yet. The badge beside the category is the
 * recipe's estimated total time (#84): "23 min+", an at-least — untimed steps
 * count as nothing in the estimate, so the real cook takes longer. */
export const Swiping: Story = {
  args: { status: "swiping", card: cards[0], participants: 2, shareUrl: share },
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

/**
 * Somebody stepped out mid-swipe (#96).
 *
 * More than a courtesy note: the roster is the number a recipe has to win over, so a
 * departure moves the bar *down* and can complete an agreement that was one holdout
 * away. A recipe winning the instant that number dropped has to read as "Mel left",
 * never as the app deciding by itself — so the person who moved it is named, in
 * their own colour, and stays named. It does not fade, because it explains the
 * target everyone is swiping against for the rest of the session.
 */
export const SomeoneLeft: Story = {
  args: {
    status: "swiping",
    card: cards[2],
    participants: 2,
    shareUrl: share,
    departed: { telegram_user_id: "5150", username: "mel" },
  },
};

/**
 * The last person left, so the plan is over (#96).
 *
 * An empty plan is nobody's meal: the backend closes it and the link stops resolving,
 * so anyone still watching is told rather than left swiping into a channel that no
 * longer exists. A Notice rather than the Alert, because nothing failed — and no
 * Leave, because there is nothing left to leave.
 */
export const PlanEnded: Story = {
  args: { status: "ended" },
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
  args: { status: "error", error: "Could not reach the room (502)." },
};
