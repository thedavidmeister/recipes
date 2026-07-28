import type { Meta, StoryObj } from "@storybook/sveltekit";
import UserName from "./UserName.svelte";

const meta = {
  title: "recipes/UserName",
  component: UserName,
} satisfies Meta<typeof UserName>;
export default meta;

type Story = StoryObj<typeof meta>;

/** The usual case: a handle, and the colour their Telegram id lands on. */
export const Named: Story = {
  args: { user: { telegram_user_id: "4242", username: "dave" } },
};

/** Another person, so the point is visible: the colour follows the id, and nobody
 * chose it. */
export const AnotherPerson: Story = {
  args: { user: { telegram_user_id: "5150", username: "mel" } },
};

/** A Telegram account need not have a username. The numeric id is the label then —
 * it is the identity either way, so the colour is unchanged by the missing handle. */
export const NoUsername: Story = {
  args: { user: { telegram_user_id: "9317", username: null } },
};

/** Named inside a sentence rather than as a line of a list (#96) — the plan lobby
 * says who leaving would hand the plan to. The list form is a block, so mid-prose it
 * would take the whole line and strand the rest of the sentence underneath; this one
 * sits on the baseline and the words carry on around it. */
export const InSentence: Story = {
  args: { user: { telegram_user_id: "5150", username: "mel" }, inline: true },
};
