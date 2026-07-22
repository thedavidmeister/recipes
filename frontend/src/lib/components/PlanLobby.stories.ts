import type { Meta, StoryObj } from "@storybook/sveltekit";
import PlanLobby from "./PlanLobby.svelte";

const invite = "https://recipes.lehlehleh.com/pick/8f2a1c4e9b7d";

const meta = {
  title: "recipes/PlanLobby",
  component: PlanLobby,
  args: { onStart: () => {} },
} satisfies Meta<typeof PlanLobby>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Alone, which is a complete meal plan: start whenever, or invite someone first. */
export const Solo: Story = {
  args: {
    status: "ready",
    host: true,
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** Three deciding — the number a recipe now has to win over. */
export const Gathered: Story = {
  args: {
    status: "ready",
    host: true,
    inviteLink: invite,
    voters: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: null },
      { telegram_user_id: "5150", username: "mel" },
    ],
  },
};

/** A guest waits: starting is the host's call, so a late arrival cannot close the
 * door on whoever is still inviting people. */
export const Guest: Story = {
  args: {
    status: "ready",
    host: false,
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
