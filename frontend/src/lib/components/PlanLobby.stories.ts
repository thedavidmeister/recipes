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
    onCanMake: () => {},
  },
} satisfies Meta<typeof PlanLobby>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Alone, which is a complete meal plan: start whenever, or invite someone first.
 * Every plan is born for dinner (#114); the host's picker is there to say otherwise. */
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

/** The host has capped the plan to 30 minutes (#80): the bucket reads selected and
 * the honest fine print is attached — the estimate is a lower bound, and recipes
 * without timings still show. */
export const TimeCapped: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    inviteLink: invite,
    cap: 1800,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** A plan for a kitchen (#82): the can-make row is offered, and sits unbounded —
 * "Anything" — which is where every plan starts. A kitchen that has listed nothing
 * can make nothing, so a bound that was on by default would deal an empty deck. */
export const KitchenUnbounded: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    hasKitchen: true,
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** The host has bounded the plan to what the kitchen can make (#82). The note is
 * part of the control, not a footnote: this leaves out recipes nobody has read for
 * equipment yet, so the deck is narrower than what the kitchen could really cook. */
export const KitchenCanMake: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    hasKitchen: true,
    requireKitchenEquipment: true,
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** Both bounds at once, which is how a weeknight plan actually reads: half an hour,
 * and only what this kitchen has the tools for (#80 + #82). Each says what it costs
 * on its own line rather than merging into one hedge. */
export const CappedAndCanMake: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    hasKitchen: true,
    cap: 1800,
    requireKitchenEquipment: true,
    inviteLink: invite,
    voters: [{ telegram_user_id: "4242", username: "dave" }],
  },
};

/** A guest reads the kitchen bound they will be swiping within — shown, not
 * settable, like the time cap (#82). Someone wondering where half the recipes went
 * gets the answer on the page instead of a theory. */
export const GuestSeesTheKitchenBound: Story = {
  args: {
    status: "ready",
    host: false,
    hostId: "4242",
    mealType: "dinner",
    requireKitchenEquipment: true,
    voters: [
      { telegram_user_id: "4242", username: "dave" },
      { telegram_user_id: "9317", username: "mel" },
    ],
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
