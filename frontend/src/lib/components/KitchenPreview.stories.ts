import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenPreview from "./KitchenPreview.svelte";

const meta = {
  title: "recipes/KitchenPreview",
  component: KitchenPreview,
} satisfies Meta<typeof KitchenPreview>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The whole page: the owners, the way to start a meal, and the meals already here —
 * all legible over the photograph. */
export const Default: Story = { args: {} };
