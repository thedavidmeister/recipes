import type { StorybookConfig } from "@storybook/sveltekit";

const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(js|ts)"],
  framework: {
    name: "@storybook/sveltekit",
    options: {},
  },
  core: {
    disableTelemetry: true,
  },
};

export default config;
