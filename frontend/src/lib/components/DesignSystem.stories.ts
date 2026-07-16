import type { Meta, StoryObj } from "@storybook/sveltekit";
import DesignSystem from "./DesignSystem.svelte";

// The living style guide, as one screenshottable page. `satisfies` (not an
// annotation) keeps StoryObj arg inference working.
const meta = {
  title: "recipes/Design System",
  component: DesignSystem,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof DesignSystem>;
export default meta;

type Story = StoryObj<typeof meta>;

export const Overview: Story = {};
