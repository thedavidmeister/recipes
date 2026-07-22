import type { Meta, StoryObj } from "@storybook/sveltekit";
import MusicSwitch from "./MusicSwitch.svelte";

const meta = {
  title: "recipes/MusicSwitch",
  component: MusicSwitch,
  args: { onToggle: () => {}, onVolume: () => {} },
} satisfies Meta<typeof MusicSwitch>;
export default meta;

type Story = StoryObj<typeof meta>;

/** Playing, and obviously so: the switch fills in and the level appears beside it. */
export const Playing: Story = { args: { playing: true, volume: 0.5 } };

/** Turned down but still going — the level is where you left it. */
export const Quiet: Story = { args: { playing: true, volume: 0.12 } };

/** Switched off: no level, because there is nothing to level. */
export const Off: Story = { args: { playing: false, volume: 0.5 } };
