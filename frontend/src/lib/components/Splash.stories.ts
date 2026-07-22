import type { Meta, StoryObj } from "@storybook/sveltekit";
import Splash from "./Splash.svelte";

const meta = {
  title: "recipes/Splash",
  component: Splash,
  args: { onStart: () => {} },
} satisfies Meta<typeof Splash>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The way in: the room, the arc, and one thing to press. */
export const Default: Story = { args: {} };
