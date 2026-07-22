import type { StorybookConfig } from "@storybook/sveltekit";

const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(js|ts)"],
  // Serve the app's static assets, so a component that references one (the kitchens
  // backdrop, /kitchen.jpg) renders here the way it does in the app instead of as a
  // broken image — which would otherwise get blessed into a baseline.
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
