import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenRename from "./KitchenRename.svelte";

const meta = {
  title: "recipes/KitchenRename",
  component: KitchenRename,
  args: { onRename: () => {} },
} satisfies Meta<typeof KitchenRename>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Arrives holding the name it has, so a rename is an edit rather than a retype. */
export const Default: Story = { args: { current: "dave's kitchen" } };

/** It didn't land — the reason stays, and so does what you typed. */
export const Failed: Story = {
  args: {
    current: "dave's kitchen",
    error: "only the owner can rename this kitchen (403)",
  },
};
