import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenList from "./KitchenList.svelte";
import { kitchenList } from "$lib/fixtures";

const meta = {
  title: "recipes/KitchenList",
  component: KitchenList,
} satisfies Meta<typeof KitchenList>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The kitchens you're in — yours, and ones a friend invited you to. */
export const Ready: Story = {
  args: { status: "ready", kitchens: kitchenList() },
};

/** Nothing yet: the create field is the whole page. */
export const Empty: Story = { args: { status: "ready", kitchens: [] } };

/** A create that didn't land — the reason stays, and so does what you typed. */
export const ActionFailed: Story = {
  args: {
    status: "ready",
    kitchens: kitchenList(),
    actionError: "could not create the kitchen (502)",
  },
};

export const Pending: Story = { args: { status: "pending" } };

export const Error: Story = {
  args: { status: "error", error: "could not load your kitchens (502)" },
};
