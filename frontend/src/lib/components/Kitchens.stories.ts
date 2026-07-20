import type { Meta, StoryObj } from "@storybook/sveltekit";
import Kitchens from "./Kitchens.svelte";
import { kitchenList, kitchenDetail } from "$lib/fixtures";

const meta = {
  title: "recipes/Kitchens",
  component: Kitchens,
} satisfies Meta<typeof Kitchens>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A kitchen open in full — members, invite, equipment, and pantry. */
export const Ready: Story = {
  args: {
    status: "ready",
    kitchens: kitchenList(),
    selected: kitchenDetail(),
    inviteLink: "https://recipes.lehlehleh.com/kitchens?join=a1b2c3d4e5f6a7b8",
  },
};

/** No kitchens yet — the create/join entry point. */
export const Empty: Story = {
  args: { status: "ready", kitchens: [], selected: null },
};

/** Loading the list. */
export const Pending: Story = {
  args: { status: "pending" },
};

/** The list could not be loaded. */
export const Error: Story = {
  args: { status: "error", error: "could not load your kitchens (502)" },
};
