import type { Meta, StoryObj } from "@storybook/sveltekit";
import CategoryPicker from "./CategoryPicker.svelte";
import { categories } from "$lib/fixtures";

const meta = {
  title: "recipes/CategoryPicker",
  component: CategoryPicker,
} satisfies Meta<typeof CategoryPicker>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Nothing chosen — the trigger shows the placeholder. */
export const Empty: Story = {
  args: { categories },
};

export const Selected: Story = {
  args: { categories, value: "Seafood" },
};
