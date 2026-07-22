import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchensPreview from "./KitchensPreview.svelte";

const meta = {
  title: "recipes/KitchensPreview",
  component: KitchensPreview,
} satisfies Meta<typeof KitchensPreview>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The whole page: content legible over the photograph and its scrim. */
export const Default: Story = { args: {} };
