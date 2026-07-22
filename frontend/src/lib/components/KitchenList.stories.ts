import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenList from "./KitchenList.svelte";
import { kitchenList } from "$lib/fixtures";

const meta = {
  title: "recipes/KitchenList",
  component: KitchenList,
} satisfies Meta<typeof KitchenList>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The kitchens you're in: your primary at the top, then any you made, then the ones
 * a friend invited you to. There is no empty counterpart — the primary always
 * exists. */
export const Ready: Story = {
  args: { status: "ready", kitchens: kitchenList() },
};

/** An invite link that wouldn't redeem — the reason lands here, on the page it sent
 * you to. */
export const ActionFailed: Story = {
  args: {
    status: "ready",
    kitchens: kitchenList(),
    actionError: "that invite has already been used",
  },
};

export const Pending: Story = { args: { status: "pending" } };

export const Error: Story = {
  args: { status: "error", error: "could not load your kitchens (502)" },
};
