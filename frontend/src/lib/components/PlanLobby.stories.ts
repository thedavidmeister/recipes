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
    onLeave: () => {},
    // Every plan is born capped at half an hour (#163), so that is the shared
    // starting state and the stories below vary from it, rather than each one
    // having to restate what a lobby looks like when you make one. A story about
    // some other cap says so; `Uncapped` is the host having widened it.
    cap: 1800,
  },
} satisfies Meta<typeof PlanLobby>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * Alone, which is a complete meal plan: start whenever, or invite someone first.
 * A plan is born for dinner (#114) and capped at half an hour (#163) — this is the
 * lobby exactly as making one hands it to you, with both pickers already saying
 * something and there for the host to say otherwise.
 *
 * Also the last-person-out case for leaving (#96): with nobody else here, walking
 * away ends the plan rather than shrinking it, so the button says so before it is
 * tapped instead of a modal asking afterwards.
 */
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

/**
 * Three deciding — the number a recipe now has to win over.
 *
 * And the host-leaving case (#96): the plan passes to the next person in the room
 * rather than being taken down, so the note under Leave names who would get it. A
 * host who could not leave would be trapped in their own plan, and one who took it
 * down with them would cancel a meal other people are having.
 */
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

/** Dinner with things alongside (#114): the host toggled dessert and a side on, so
 * the chosen pills fill in — both of them, the whole tier — and the heading gains
 * its quiet "with dessert & side" line; the room decides the dinner, these come
 * with it. */
export const WithAdditions: Story = {
  args: {
    status: "ready",
    host: true,
    hostId: "4242",
    mealType: "dinner",
    additions: ["dessert", "side"],
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
    additions: ["side"],
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
  args: { status: "error", error: "Couldn't open this meal plan (404)." },
};

/**
 * The last person left, so the plan is over (#96).
 *
 * This lives on the lobby and nowhere else, because a plan can only be emptied
 * *before* it starts — the roster closes at the start in both directions — so the
 * swipe view can never be the screen looking at one. Reached by a client still in
 * the room when the last decider walks out: a second tab of the leaver's own, or
 * somebody watching a lobby they never got seated into.
 *
 * Said plainly rather than as an alert: nothing failed, there is simply nothing
 * left to decide — and no Start and no Leave, because there is no plan to start
 * and nothing left to leave.
 *
 * The meal is named because it is always known here: a client only learns a plan
 * ended by being in its room, which means it had already read the lobby. (`Error`
 * leaves it out for the opposite reason — there, the read is what failed.)
 */
export const PlanEnded: Story = {
  args: { status: "ended", mealType: "dinner" },
};

/**
 * The same lobby with a host whose colour is `honey` — the palest slot there is.
 *
 * Worth its own story because it is the one that would break a design leaning on
 * the colour to say "chosen": `honey-500` measures 2.0:1 on cream, well under the
 * 3:1 a control's boundary needs. Chosen-ness is the achromatic `stone-200` fill,
 * the cocoa outline and `aria-pressed`; the dot alone carries *whose*, which is the
 * same job it does on `Start`. So a pale host reads exactly as well as a dark one,
 * and this story is where that stays true.
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
