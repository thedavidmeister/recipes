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
    error: "could not check session (503)",
  },
};
