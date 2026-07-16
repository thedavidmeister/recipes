import type { Meta, StoryObj } from "@storybook/sveltekit";
import Nav from "./Nav.svelte";

// `satisfies` (not an annotation): StoryObj<typeof meta> infers args from
// `component`, which only works when typeof meta keeps the literal shape.
const meta = {
  title: "recipes/Nav",
  component: Nav,
} satisfies Meta<typeof Nav>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Each section, so the active treatment is reviewable without clicking. */
export const Pick: Story = { args: { current: "pick" } };
export const Buy: Story = { args: { current: "buy" } };
export const Cook: Story = { args: { current: "cook" } };
export const Joy: Story = { args: { current: "joy" } };
