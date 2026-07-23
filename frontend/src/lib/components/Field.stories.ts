import type { Meta, StoryObj } from "@storybook/sveltekit";
import Field from "./Field.svelte";

const meta = {
  title: "recipes/Field",
  component: Field,
} satisfies Meta<typeof Field>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A labelled thing to type in — the label stays put while you type. */
export const Default: Story = {
  args: { id: "kitchen-name", label: "What do you call it?", value: "", placeholder: "Home" },
};

/** Filled in. */
export const Filled: Story = {
  args: { id: "kitchen-name", label: "What do you call it?", value: "The Shed" },
};
