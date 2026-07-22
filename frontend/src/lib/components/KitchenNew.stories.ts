import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenNew from "./KitchenNew.svelte";

const meta = {
  title: "recipes/KitchenNew",
  component: KitchenNew,
  args: { onCreate: () => {} },
} satisfies Meta<typeof KitchenNew>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Naming it is the whole page. */
export const Default: Story = { args: {} };

/** It didn't land — the reason stays, and so does what you typed. */
export const Failed: Story = {
  args: { error: "could not create the kitchen (502)" },
};
