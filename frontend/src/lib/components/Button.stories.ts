import type { Meta, StoryObj } from "@storybook/sveltekit";
import Button from "./Button.svelte";

const meta = {
  title: "recipes/Button",
  component: Button,
} satisfies Meta<typeof Button>;
export default meta;

type Story = StoryObj<typeof meta>;

/** To act. Cream surface, the border doing the shaping, and colour only in the dot —
 * which is what keeps a page of actions from reading as a wall of paint. */
export const Primary: Story = {
  args: { variant: "primary", dot: "pesto", children: "Cook this" as never },
};

/** To choose otherwise. Same shape, quieter, no dot. */
export const Secondary: Story = {
  args: { variant: "secondary", children: "Maybe later" as never },
};

/** To leave. No border at all — an exit should not compete with the way forward. */
export const Quiet: Story = {
  args: { variant: "quiet", children: "skip" as never },
};

/** Navigation that reads as an action: an anchor wearing the same clothes. */
export const AsLink: Story = {
  args: { href: "/kitchens/new", dot: "cocoa", children: "New kitchen" as never },
};
