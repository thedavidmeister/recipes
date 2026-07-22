import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenItems from "./KitchenItems.svelte";
import { kitchenDetail } from "$lib/fixtures";

const meta = {
  title: "recipes/KitchenItems",
  component: KitchenItems,
} satisfies Meta<typeof KitchenItems>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The same page serves equipment and pantry; only the words differ. */
export const Equipment: Story = {
  args: {
    status: "ready",
    title: "Equipment",
    items: kitchenDetail().equipment,
    placeholder: "Add equipment (blender, wok…)",
    backHref: "/kitchens/k1",
  },
};

export const Pantry: Story = {
  args: {
    status: "ready",
    title: "Pantry",
    items: kitchenDetail().pantry,
    placeholder: "Add to the pantry (rice, eggs…)",
    backHref: "/kitchens/k1",
  },
};

/** Nothing tracked yet. */
export const Empty: Story = {
  args: {
    status: "ready",
    title: "Equipment",
    items: [],
    placeholder: "Add equipment (blender, wok…)",
    backHref: "/kitchens/k1",
  },
};
