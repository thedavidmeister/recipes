import type { Meta, StoryObj } from "@storybook/sveltekit";
import Pick from "./Pick.svelte";
import { recipeCards } from "$lib/fixtures";

const meta = {
  title: "recipes/Pick",
  component: Pick,
} satisfies Meta<typeof Pick>;
export default meta;

type Story = StoryObj<typeof meta>;

const cards = recipeCards();
const share = "https://recipes.lehlehleh.com/pick/ab12cd34ef56";

/** A card up to vote on, no consensus yet. */
export const Swiping: Story = {
  args: { status: "swiping", card: cards[0], participants: 2, shareUrl: share },
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
  args: { status: "error", error: "Could not reach the room (502)." },
};
