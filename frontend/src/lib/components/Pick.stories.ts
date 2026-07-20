import type { Meta, StoryObj } from "@storybook/sveltekit";
import Pick from "./Pick.svelte";
import { matches, recipeCards } from "$lib/fixtures";

const meta = {
  title: "recipes/Pick",
  component: Pick,
} satisfies Meta<typeof Pick>;
export default meta;

type Story = StoryObj<typeof meta>;

const cards = recipeCards();
const share = "https://recipes.lehlehleh.com/pick/ab12cd34ef56";

/** A card up to vote on, no match yet. */
export const Swiping: Story = {
  args: { status: "swiping", card: cards[0], participants: 2, shareUrl: share },
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
 * Consensus: everyone said yes to a recipe — that's the pick, surfaced inline
 * while the swipe keeps going for another.
 */
export const Matched: Story = {
  args: {
    status: "swiping",
    card: cards[1],
    matches: matches(),
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
  args: { status: "error", error: "Could not reach the room (502)." },
};
