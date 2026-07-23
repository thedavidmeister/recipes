import type { Meta, StoryObj } from "@storybook/sveltekit";
import { text } from "./story-text";
import Panel from "./Panel.svelte";

const meta = {
  title: "recipes/Panel",
  component: Panel,
} satisfies Meta<typeof Panel>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The surface everything readable sits on. */
export const Default: Story = { args: { children: text("Anything legible") } };
