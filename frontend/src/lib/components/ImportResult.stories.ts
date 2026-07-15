import type { Meta, StoryObj } from "@storybook/sveltekit";
import ImportResult from "./ImportResult.svelte";
import { recipe } from "$lib/fixtures";

// `satisfies` (not an annotation): StoryObj<typeof meta> infers args from
// `component`, which only works when typeof meta keeps the literal shape.
const meta = {
  title: "recipes/ImportResult",
  component: ImportResult,
} satisfies Meta<typeof ImportResult>;
export default meta;

type Story = StoryObj<typeof meta>;

export const Pending: Story = {
  args: { pending: true },
};

export const Saved: Story = {
  args: { result: { kind: "saved", recipe: recipe() } },
};

/** A page whose schema.org data is too thin to store. */
export const Incomplete: Story = {
  args: {
    result: {
      kind: "incomplete",
      recipe: recipe({ ingredients: [], instructions: "" }),
    },
  },
};

/** The acceptance case: a real page that simply has no Recipe on it. */
export const NoRecipe: Story = {
  args: { result: { kind: "no-recipe", url: "https://example.com/about" } },
};

export const InvalidUrl: Story = {
  args: { result: { kind: "invalid-url", message: "That doesn’t look like a URL." } },
};

/** Unreachable, non-2xx, or blocked by the proxy's SSRF guard. */
export const FetchFailed: Story = {
  args: { result: { kind: "fetch-failed", message: "fetch failed (403)" } },
};

export const SaveFailed: Story = {
  args: {
    result: {
      kind: "save-failed",
      recipe: recipe(),
      message: "save failed (500)",
    },
  },
};
