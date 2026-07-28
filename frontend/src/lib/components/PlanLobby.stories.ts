import type { Meta, StoryObj } from "@storybook/sveltekit";
import PlanLobby from "./PlanLobby.svelte";

const invite = "https://recipes.lehlehleh.com/pick/8f2a1c4e9b7d";

const meta = {
  title: "recipes/PlanLobby",
  component: PlanLobby,
  args: {
    onStart: () => {},
    onMealType: () => {},
    onAdditions: () => {},
    onCap: () => {},
    // Every plan is born capped at half an hour (#163), so that is the shared
    // starting state and the stories below vary from it, rather than each one
    // having to restate what a lobby looks like when you make one. A story about
    // some other cap says so; `Uncapped` is the host having widened it.
    cap: 1800,
  },
} satisfies Meta<typeof PlanLobby>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Alone, which is a complete meal plan: start whenever, or invite someone first.
 * A plan is born for dinner (#114) and capped at half an hour (#163) — this is the
 * lobby exactly as making one hands it to you, with both pickers already saying
 * something and there for the host to say otherwise. */
export const Solo: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
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
    hostId: "4242",
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
    hostId: "4242",
    mealType: "breakfast",
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** Dinner with things alongside (#114): the host toggled dessert and drinks on,
 * so the chosen pills fill in and the heading gains its quiet "with dessert &
 * drink" line — the room decides the dinner; these come with it. */
export const WithAdditions: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    additions: ["dessert", "drink"],
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
    hostId: "4242",
    mealType: "dinner",
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
    candidates: [
      { telegram_user_id: "5150", username: "mel" },
      { telegram_user_id: "6161", username: "sam" },
    ],
  },
};

/** The host has the afternoon, so they widened the plan to "Any" (#163) — the far
 * end of the row, one tap from where the plan started. Nothing is selected among
 * the numbers and the honest fine print goes with them: there is no bound left to
 * be honest about. This is the state that used to be every plan's first moment, and
 * declaring it is what keeps the default from becoming a floor. */
export const Uncapped: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    inviteLink: invite,
    cap: null,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** A guest in a capped plan sees the bound they will be swiping within — shown,
 * not settable: the cap is the host's call (#80). */
export const GuestSeesTheCap: Story = {
  args: {
    status: "ready",
    host: false,
    hostId: "4242",
    mealType: "dinner",
    cap: 3600,
    voters: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: "mel" },
    ],
  },
};

/** A guest waits: starting is the host's call, so a late arrival cannot close the
 * door on whoever is still inviting people. The heading still names the meal and
 * what comes with it — a guest reads both, only the host changes them. */
export const Guest: Story = {
  args: {
    status: "ready",
    host: false,
    hostId: "4242",
    mealType: "snack",
    additions: ["drink"],
    // Pinned open rather than inheriting the born default, so the guest view is
    // declared both ways: `GuestSeesTheCap` is the bound one, this is the plan a
    // host has widened, where there is no line to read at all.
    cap: null,
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

/**
 * The same lobby with a host whose colour is `honey` — the palest slot there is.
 *
 * Worth its own story because it is the one that would break a design leaning on
 * the colour to say "chosen": `honey-500` measures 2.0:1 on cream, well under the
 * 3:1 a control's boundary needs. Here the cocoa outline and `aria-pressed` carry
 * chosen-ness and the tint only carries *whose*, so the pale host reads exactly as
 * well as a dark one.
 */
export const HostWearsThePalestColour: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "3141",
    mealType: "lunch",
    additions: ["side"],
    cap: 1800,
    inviteLink: invite,
    voters: [{ telegram_user_id: "3141", username: "kit" }],
  },
};
