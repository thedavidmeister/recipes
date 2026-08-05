import type { StorybookConfig } from "@storybook/sveltekit";

const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(js|ts)"],
  // Serve the app's static assets, so a component that references one (the kitchens
  // backdrop's /kitchen-*.jpg ladder) renders here the way it does in the app instead
  // of as a broken image — which would otherwise get blessed into a baseline. The whole
  // ladder has to be here, not just the rung the fence picks, or the browser could not
  // make the pick.
  staticDirs: ["../static"],
  framework: {
    name: "@storybook/sveltekit",
    options: {},
  },
  core: {
    disableTelemetry: true,
  },
};

export default config;
