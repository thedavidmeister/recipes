import type { Meta, StoryObj } from "@storybook/sveltekit";
import Winners from "./Winners.svelte";
import { winners } from "$lib/fixtures";

const meta = {
  title: "recipes/Winners",
  component: Winners,
} satisfies Meta<typeof Winners>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Most-liked: everything with a yes, ranked; the leader is ringed. */
export const Plurality: Story = {
  args: { condition: "plurality", participants: 3, candidates: winners() },
};

/** Everyone-agreed: only the recipe all three said yes to survives. */
export const Consensus: Story = {
  args: { condition: "consensus", participants: 3, candidates: winners() },
};

/** Consensus with a fourth voter yet to weigh in — nobody is unanimous yet. */
export const NoConsensusYet: Story = {
  args: { condition: "consensus", participants: 4, candidates: winners() },
};

/** Before anyone has swiped. */
export const Empty: Story = {
  args: { condition: "plurality", participants: 0, candidates: [] },
};
