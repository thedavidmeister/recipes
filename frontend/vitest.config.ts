import { defineConfig } from "vitest/config";

/**
 * Unit tests for the SPA's pure modules (`src/lib/*.test.ts`).
 *
 * Its own config rather than a `test` block on `vite.config.ts`, deliberately: that
 * config exists to build the app, so it loads the SvelteKit and Tailwind plugins,
 * and a test run has no use for either. What is under test here is plain TypeScript
 * — formatting and graph arithmetic — so the runner needs no DOM, no `$env`, and no
 * SvelteKit sync step, and pulling the app's build pipeline in would only give it
 * ways to fail that have nothing to do with the code being tested.
 *
 * Component behaviour is *not* tested here. Every UI state is a Storybook story and
 * the visual fence renders it (see README/CLAUDE.md) — that is the project's answer
 * for anything that renders. This covers the logic underneath, which a screenshot
 * cannot pin: `formatEstimate` returning null for an unknown time looks identical to
 * a bug that drops the badge.
 */
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
