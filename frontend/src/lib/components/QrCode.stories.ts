import type { Meta, StoryObj } from "@storybook/sveltekit";
import QrCode from "./QrCode.svelte";

const meta = {
  title: "recipes/QrCode",
  component: QrCode,
} satisfies Meta<typeof QrCode>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A kitchen invite — scan it off a screen to join. */
export const Invite: Story = {
  args: {
    value: "https://recipes.lehlehleh.com/kitchens?join=a1b2c3d4e5f6a7b8",
    label: "Scan to join Home",
  },
};
