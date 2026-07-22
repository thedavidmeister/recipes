import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenBackdrop from "./KitchenBackdrop.svelte";

const meta = {
  title: "recipes/KitchenBackdrop",
  component: KitchenBackdrop,
} satisfies Meta<typeof KitchenBackdrop>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The photograph and its scrim, with nothing over it. */
export const Default: Story = { args: {} };
