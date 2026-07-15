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
 * mandatory (#25).
 */
export const Idle: Story = {
  args: { status: "idle" },
};

/** Boot: asking `/api/me` whether a session already exists. */
export const Checking: Story = {
  args: { status: "checking" },
};

export const Starting: Story = {
  args: { status: "starting" },
};

/**
 * The state that makes this component worth having as a story: reaching it for
 * real needs a live nonce, and *leaving* it needs someone to tap the link in
 * Telegram. The link here is a fixture — nothing is minted.
 */
export const Waiting: Story = {
  args: {
    status: "waiting",
    link: "https://t.me/lehlehlehbot?start=0000000000000000000000000000000000000000000000000000000000000000",
  },
};

/** Nobody tapped within the nonce's 15 minutes — unclickable-to by design. */
export const Expired: Story = {
  args: { status: "expired" },
};

/** The backend is down or refused. Recoverable: the user can retry. */
export const ErrorState: Story = {
  name: "Error",
  args: { status: "error", error: "could not start login (503)" },
};
