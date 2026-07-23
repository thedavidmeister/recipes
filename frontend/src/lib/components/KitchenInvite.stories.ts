import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenInvite from "./KitchenInvite.svelte";

const link = "https://recipes.lehlehleh.com/kitchens?join=a1b2c3d4e5f6a7b8";

const meta = {
  title: "recipes/KitchenInvite",
  component: KitchenInvite,
  args: { onRenew: () => {} },
} satisfies Meta<typeof KitchenInvite>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A code to scan, a link to send, and how long it lasts — the whole page. The
 * remaining time is a fixed number here, because a component that read the clock could
 * not be pinned by a baseline. */
export const Ready: Story = {
  args: { status: "ready", kitchen: "dave's kitchen", link, remaining: 7140 },
};

/** The last minute counts in seconds: coarse is fine at the top, not at the end. */
export const AlmostGone: Story = {
  args: { status: "ready", kitchen: "dave's kitchen", link, remaining: 45 },
};

/** Run out. A link left open is no use to anybody — including whoever finds it — so
 * the only thing offered is a fresh one. */
export const Expired: Story = {
  args: { status: "ready", kitchen: "dave's kitchen", link, remaining: 0 },
};

export const Pending: Story = { args: { status: "pending" } };

export const Error: Story = {
  args: { status: "error", error: "could not open this kitchen (403)" },
};
