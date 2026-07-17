import type { Meta, StoryObj } from "@storybook/sveltekit";
import Walk from "./Walk.svelte";
import { walkStops } from "$lib/fixtures";

const meta = {
  title: "recipes/Walk",
  component: Walk,
} satisfies Meta<typeof Walk>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A full walk: five real meals threaded by shared ingredients. */
export const Ready: Story = {
  args: { status: "ready", stops: walkStops() },
};

/** Loading the first walk — the skeleton of the journey to come. */
export const Pending: Story = {
  args: { status: "pending" },
};

/** Re-rolling: a walk is shown, but the button reads busy. */
export const Wandering: Story = {
  args: { status: "ready", stops: walkStops(), busy: true },
};

/** The corpus is empty (nothing ingested yet). */
export const Empty: Story = {
  args: { status: "ready", stops: [] },
};

/** The walk could not be fetched. */
export const Error: Story = {
  args: { status: "error", error: "could not walk the corpus (502)" },
};
