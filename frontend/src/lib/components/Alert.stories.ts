import type { Meta, StoryObj } from "@storybook/sveltekit";
import { text } from "./story-text";
import Alert from "./Alert.svelte";

const meta = {
  title: "recipes/Alert",
  component: Alert,
} satisfies Meta<typeof Alert>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The one place the palette raises its voice. */
export const Default: Story = { args: { children: text("The pick dropped.") } };
