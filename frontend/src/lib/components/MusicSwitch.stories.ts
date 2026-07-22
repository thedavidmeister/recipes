import type { Meta, StoryObj } from "@storybook/sveltekit";
import MusicSwitch from "./MusicSwitch.svelte";

const meta = {
  title: "recipes/MusicSwitch",
  component: MusicSwitch,
  args: { onToggle: () => {} },
} satisfies Meta<typeof MusicSwitch>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Playing, and obviously so: the switch fills in and the speaker sounds. */
export const Playing: Story = { args: { playing: true } };

/** Switched off — the way back is the same button. */
export const Off: Story = { args: { playing: false } };
