import type { Preview } from "@storybook/sveltekit";
// Tailwind, so stories render with the app's real styles.
import "../src/app.css";

const preview: Preview = {
  parameters: {
    layout: "fullscreen",
  },
};

export default preview;
