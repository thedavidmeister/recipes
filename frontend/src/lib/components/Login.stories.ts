import type { Meta, StoryObj } from "@storybook/sveltekit";
import Login from "./Login.svelte";

// `satisfies` (not an annotation): StoryObj<typeof meta> infers args from
// `component`, which only works when typeof meta keeps the literal shape.
const meta = {
  title: "recipes/Login",
  component: Login,
} satisfies Meta<typeof Login>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * A visitor with no session — the first screen anyone sees, since auth is
 * mandatory (#25). It points at the bot and nothing more: the login is started
 * by messaging the bot, never by this page.
 */
export const Idle: Story = {
  args: { status: "idle", link: "https://t.me/lehlehlehbot" },
};

/**
 * Reached from an invite — a scanned QR, most of the time (#206). A scan opens the
 * system browser, which holds no session however signed in the phone's Telegram is,
 * so this is the screen an invite lands on. The bot link is a deep link carrying the
 * plan, and the screen says so: signing in comes back here, rather than dropping the
 * invite and landing at home.
 */
export const FromAnInvite: Story = {
  args: {
    status: "idle",
    returning: true,
    // `L3BpY2sv…` is `/pick/9f4b2c1d8e7a6053` in base64url — Telegram's `?start=`
    // alphabet. See `$lib/destination`.
    link: "https://t.me/lehlehlehbot?start=L3BpY2svOWY0YjJjMWQ4ZTdhNjA1Mw",
  },
};

/** Boot: asking `/api/me` whether a session already exists. */
export const Checking: Story = {
  args: { status: "checking", link: "https://t.me/lehlehlehbot" },
};

/** The backend is unreachable. Not clickable-to, hence a story. */
export const ErrorState: Story = {
  name: "Error",
  args: {
    status: "error",
    link: "https://t.me/lehlehlehbot",
    error: "Couldn't check whether you're signed in (503).",
  },
};
