import type { Meta, StoryObj } from "@storybook/sveltekit";
import Kitchen from "./Kitchen.svelte";
import { kitchenDetail } from "$lib/fixtures";

const meta = {
  title: "recipes/Kitchen",
  component: Kitchen,
} satisfies Meta<typeof Kitchen>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Who's in it, how to invite, and the way through to equipment and pantry. */
export const Ready: Story = {
  args: {
    status: "ready",
    kitchen: kitchenDetail(),
  },
};

export const Pending: Story = { args: { status: "pending" } };

/** A kitchen that won't open — removed from it, or an id that no longer exists. */
export const Error: Story = {
  args: { status: "error", error: "not a member of this kitchen (403)" },
};
