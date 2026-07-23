import type { Meta, StoryObj } from "@storybook/sveltekit";
import { text } from "./story-text";
import RowLink from "./RowLink.svelte";

const meta = {
  title: "recipes/RowLink",
  component: RowLink,
} satisfies Meta<typeof RowLink>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The app's main way of moving: press a row, go somewhere. */
export const Default: Story = {
  args: { href: "/kitchens/k1/pantry", children: text("Pantry") },
};

/** With a count, which is most of what these rows have to say. */
export const WithCount: Story = {
  args: { href: "/kitchens/k1/equipment", trailing: "4", children: text("Equipment") },
};
