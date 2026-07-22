import type { Meta, StoryObj } from "@storybook/sveltekit";
import PickBackdrop from "./PickBackdrop.svelte";

const meta = {
  title: "recipes/PickBackdrop",
  component: PickBackdrop,
} satisfies Meta<typeof PickBackdrop>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The room the swiping happens in, with nothing over it. */
export const Default: Story = { args: {} };
