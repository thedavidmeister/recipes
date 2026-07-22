import type { Meta, StoryObj } from "@storybook/sveltekit";
import PickPreview from "./PickPreview.svelte";

const meta = {
  title: "recipes/PickPreview",
  component: PickPreview,
} satisfies Meta<typeof PickPreview>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The swipe over the room it happens in — the legibility check the fence can see. */
export const Default: Story = { args: {} };
