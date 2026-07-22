import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenTalk from "./KitchenTalk.svelte";

const meta = {
  title: "recipes/KitchenTalk",
  component: KitchenTalk,
} satisfies Meta<typeof KitchenTalk>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Two friends talking; the pot is on and nobody is watching it. */
export const Default: Story = { args: {} };
