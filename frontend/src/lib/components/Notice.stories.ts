import type { Meta, StoryObj } from "@storybook/sveltekit";
import { text } from "./story-text";
import Notice from "./Notice.svelte";

const meta = {
  title: "recipes/Notice",
  component: Notice,
} satisfies Meta<typeof Notice>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A state rather than content: waiting, empty, or explaining itself. */
export const Default: Story = { args: { children: text("Starting a pick…") } };
