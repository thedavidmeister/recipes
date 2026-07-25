import type { Meta, StoryObj } from "@storybook/sveltekit";
import PlanLobby from "./PlanLobby.svelte";

const invite = "https://recipes.lehlehleh.com/pick/8f2a1c4e9b7d";

const meta = {
  title: "recipes/PlanLobby",
  component: PlanLobby,
  args: { onStart: () => {}, onMealType: () => {} },
} satisfies Meta<typeof PlanLobby>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Alone, which is a complete meal plan: start whenever, or invite someone first.
 * Every plan is born for dinner (#114); the host's picker is there to say otherwise. */
export const Solo: Story = {
  args: {
    status: "ready",
    host: true,
    mealType: "dinner",
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** Three deciding — the number a recipe now has to win over. */
export const Gathered: Story = {
  args: {
    status: "ready",
    host: true,
    mealType: "dinner",
    inviteLink: invite,
    voters: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: null },
      { telegram_user_id: "5150", username: "mel" },
    ],
  },
};

/** The host flicked the plan to breakfast (#114): the heading says so for everyone,
 * and the picker shows breakfast as the chosen pill. */
export const Breakfast: Story = {
  args: {
    status: "ready",
    host: true,
    mealType: "breakfast",
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** In a kitchen: its members who are not yet deciding are offered as one-tap adds,
 * so the host does not have to send a link to people already in the kitchen (#72). */
export const KitchenMembers: Story = {
  args: {
    status: "ready",
    host: true,
    mealType: "dinner",
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
    candidates: [
      { telegram_user_id: "5150", username: "mel" },
      { telegram_user_id: "6161", username: "sam" },
    ],
  },
};

/** A guest waits: starting is the host's call, so a late arrival cannot close the
 * door on whoever is still inviting people. The heading still names the meal —
 * a guest reads it, only the host changes it. */
export const Guest: Story = {
  args: {
    status: "ready",
    host: false,
    mealType: "snack",
    voters: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: "mel" },
    ],
  },
};

export const Pending: Story = { args: { status: "pending" } };

export const Error: Story = {
  args: { status: "error", error: "could not open this meal plan (404)" },
};
