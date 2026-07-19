import type { Meta, StoryObj } from "@storybook/sveltekit";
import Decider from "./Decider.svelte";
import { recipeCards } from "$lib/fixtures";

const meta = {
  title: "recipes/Decider",
  component: Decider,
} satisfies Meta<typeof Decider>;
export default meta;

type Story = StoryObj<typeof meta>;

const cards = recipeCards();
const share = "https://recipes.lehlehleh.com/pick/ab12cd34ef56";

/** A card up to vote on, mid-session. */
export const Swiping: Story = {
  args: {
    status: "swiping",
    card: cards[0],
    inTheRunning: 3,
    participants: 2,
    shareUrl: share,
  },
};

/** Joining: the socket is opening and the tally is loading. */
export const Connecting: Story = {
  args: { status: "connecting", shareUrl: share },
};

/** The socket dropped (idle close / spin-down); the banner shows while it re-opens. */
export const Reconnecting: Story = {
  args: {
    status: "reconnecting",
    card: cards[1],
    inTheRunning: 5,
    participants: 3,
    shareUrl: share,
  },
};

/** All caught up — nothing to swipe until a peer surfaces more. */
export const CaughtUp: Story = {
  args: {
    status: "empty",
    inTheRunning: 4,
    participants: 3,
    shareUrl: share,
  },
};

/** Right after copying the invite link. */
export const LinkCopied: Story = {
  args: {
    status: "swiping",
    card: cards[2],
    inTheRunning: 2,
    participants: 2,
    shareUrl: share,
    copied: true,
  },
};

/** The room could not be reached. */
export const Error: Story = {
  args: { status: "error", error: "Could not reach the room (502)." },
};
