import type { Meta, StoryObj } from "@storybook/sveltekit";
import Kitchen from "./Kitchen.svelte";
import { kitchenDetail } from "$lib/fixtures";

const meta = {
  title: "recipes/Kitchen",
  component: Kitchen,
} satisfies Meta<typeof Kitchen>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * The owners under the title, the start-a-meal action, and the two quiet links at the
 * bottom — settings, and the switch to another kitchen. Nothing sits above the title:
 * this page is where you land, so there is no list to go "back" to (#119).
 */
export const Ready: Story = {
  args: {
    status: "ready",
    kitchen: kitchenDetail(),
  },
};

export const Pending: Story = { args: { status: "pending" } };

/** A kitchen that won't open — removed from it, or an id that no longer exists. */
export const Error: Story = {
  args: { status: "error", error: "Couldn't open this kitchen (403)." },
};
