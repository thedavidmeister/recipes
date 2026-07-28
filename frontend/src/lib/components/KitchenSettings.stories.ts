import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenSettings from "./KitchenSettings.svelte";
import { kitchenDetail } from "$lib/fixtures";

const meta = {
  title: "recipes/KitchenSettings",
  component: KitchenSettings,
} satisfies Meta<typeof KitchenSettings>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The hub: rename, invite, equipment, pantry — each its own page, gathered here with
 * a count where a count is the thing worth saying. */
export const Ready: Story = {
  args: { status: "ready", id: "k1", kitchen: kitchenDetail() },
};

export const Pending: Story = { args: { status: "pending", id: "k1" } };

/** A kitchen that won't open — removed from it, or an id that no longer exists. */
export const Error: Story = {
  args: {
    status: "error",
    id: "k1",
    error: "Couldn't open this kitchen (403).",
  },
};
