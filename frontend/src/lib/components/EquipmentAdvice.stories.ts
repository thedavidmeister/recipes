import type { Meta, StoryObj } from "@storybook/sveltekit";
import EquipmentAdvice from "./EquipmentAdvice.svelte";

const meta = {
  title: "recipes/EquipmentAdvice",
  component: EquipmentAdvice,
} satisfies Meta<typeof EquipmentAdvice>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * The state production is actually in: one kitchen, holding nothing.
 *
 * Every number here was measured against the corpus (790 recipes read for equipment,
 * 154 distinct items) rather than invented. Recipes name a median of six items, so
 * nothing a bare kitchen buys finishes one on its own — the first lines really do read
 * zero, and the page says why instead of pretending otherwise.
 */
export const AnEmptyKitchen: Story = {
  args: {
    status: "ready",
    advice: {
      additions: [
        { item: "knife", unlocks: 0 },
        { item: "bowl", unlocks: 2 },
        { item: "chopping board", unlocks: 6 },
        { item: "oven", unlocks: 9 },
        { item: "saucepan", unlocks: 30 },
      ],
      makeable: 0,
      read: 790,
    },
  },
};

/** The headline case from #83: a kitchen one item short of a pile of recipes. Measured
 * — a kitchen holding every item in the corpus except a blender can make 762 of the 790,
 * and the blender is worth exactly 28 more. There is nothing after it, because nothing
 * else is missing. */
export const OneThingShort: Story = {
  args: {
    status: "ready",
    advice: {
      additions: [{ item: "blender", unlocks: 28 }],
      makeable: 762,
      read: 790,
    },
  },
};

/** A kitchen with the tools for everything we have read is sold nothing. */
export const NothingToAdd: Story = {
  args: {
    status: "ready",
    advice: { additions: [], makeable: 790, read: 790 },
  },
};

/**
 * No recipe in the corpus carries an equipment reading, so there is nothing to count
 * over. A different silence from the one above, and it names the gap rather than
 * implying the kitchen is complete.
 *
 * This is what production shows today: `recipes.equipment` was dropped on every write
 * until #161 fixed the upsert, so the derived corpus reads as unread until a derive
 * runs. The copy points at the other thing that gap causes — an unlimited pick — rather
 * than leaving the page looking broken on its own.
 */
export const NothingReadYet: Story = {
  args: {
    status: "ready",
    advice: { additions: [], makeable: 0, read: 0 },
  },
};

export const Pending: Story = {
  args: { status: "pending" },
};

export const Failed: Story = {
  args: { status: "error", error: "Your session has expired." },
};
