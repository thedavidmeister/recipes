import type { Meta, StoryObj } from "@storybook/sveltekit";
import KitchenItems from "./KitchenItems.svelte";
import { kitchenDetail } from "$lib/fixtures";

const meta = {
  title: "recipes/KitchenItems",
  component: KitchenItems,
} satisfies Meta<typeof KitchenItems>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The same page serves equipment and pantry; only the words differ.
 *
 * Equipment is a **picker**: `options` is everything any recipe asks for, and there is
 * no way to add something that is not on it (#81). */
export const Equipment: Story = {
  args: {
    status: "ready",
    title: "Equipment",
    items: kitchenDetail().equipment,
    placeholder: "Add equipment (blender, wok…)",
    options: [
      "baking tray",
      "blender",
      "chopping board",
      "frying pan",
      "knife",
      "mixing bowl",
      "oven",
      "saucepan",
      "whisk",
      "wok",
    ],
    backHref: "/kitchens/k1",
  },
};

/** Nothing has been read yet, so there is nothing legitimate to own. The page says so
 * rather than offering a field that would only ever be refused — the ruling working,
 * not a failure. */
export const NothingKnownYet: Story = {
  args: {
    status: "ready",
    title: "Equipment",
    items: [],
    placeholder: "Add equipment (blender, wok…)",
    options: [],
    backHref: "/kitchens/k1",
  },
};

/** The pantry is also a picker (#72), and unlike equipment its list has content today:
 * it comes from the ingredient readings, long enriched. */
export const Pantry: Story = {
  args: {
    status: "ready",
    title: "Pantry",
    items: kitchenDetail().pantry,
    placeholder: "Add to the pantry (rice, eggs…)",
    options: [
      "butter",
      "chicken",
      "egg",
      "flour",
      "garlic",
      "milk",
      "olive oil",
      "onion",
      "rice",
      "salt",
    ],
    backHref: "/kitchens/k1",
  },
};

/** Nothing tracked yet. */
export const Empty: Story = {
  args: {
    status: "ready",
    title: "Equipment",
    items: [],
    placeholder: "Add equipment (blender, wok…)",
    backHref: "/kitchens/k1",
  },
};
