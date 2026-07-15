import type { Meta, StoryObj } from "@storybook/sveltekit";
import SearchResults from "./SearchResults.svelte";
import { recipes } from "$lib/fixtures";

// `satisfies` (not an annotation): StoryObj<typeof meta> infers args from
// `component`, which only works when typeof meta keeps the literal shape.
const meta = {
  title: "recipes/SearchResults",
  component: SearchResults,
} satisfies Meta<typeof SearchResults>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Nothing searched yet — renders nothing. */
export const Idle: Story = {
  args: { status: "idle" },
};

export const Pending: Story = {
  args: { status: "pending", term: "chicken" },
};

export const ErrorState: Story = {
  name: "Error",
  args: { status: "error", term: "chicken" },
};

/** A search that matched nothing — the term is echoed back. */
export const Empty: Story = {
  args: { status: "ready", recipes: [], term: "zzzz" },
};

export const Results: Story = {
  args: { status: "ready", recipes, term: "chicken" },
};
