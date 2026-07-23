import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenInvite from "./KitchenInvite.svelte";

const meta = {
  title: "recipes/KitchenInvite",
  component: KitchenInvite,
} satisfies Meta<typeof KitchenInvite>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A code to scan and a link to send — the whole page. */
export const Ready: Story = {
  args: {
    status: "ready",
    kitchen: "dave's kitchen",
    link: "https://recipes.lehlehleh.com/kitchens?join=a1b2c3d4e5f6a7b8",
  },
};

export const Pending: Story = { args: { status: "pending" } };

export const Error: Story = {
  args: { status: "error", error: "could not open this kitchen (403)" },
};
