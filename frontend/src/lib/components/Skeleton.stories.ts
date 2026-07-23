import type { Meta, StoryObj } from "@storybook/sveltekit";
import Skeleton from "./Skeleton.svelte";

const meta = {
  title: "recipes/Skeleton",
  component: Skeleton,
} satisfies Meta<typeof Skeleton>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The shape of something that has not arrived. */
export const Card: Story = { args: {} };

/** Pill-shaped, for a line of text rather than a block. */
export const Pill: Story = { args: { rounded: "pill" } };
