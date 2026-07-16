#!/usr/bin/env node
/**
 * Story-coverage enforcement.
 *
 * The project's rule is "every UI state is a Storybook story", not something you
 * reach by driving the live app — states like error/empty/waiting are
 * impractical to click to and get worse as they multiply. That rule is only real
 * if it is checked: every component ships with a story, so the states stay
 * declarable and reviewable.
 *
 * Checks that each `lib/components/*.svelte` has a sibling `*.stories.ts`.
 */
import { readdirSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const DIR = join(ROOT, "src/lib/components");

const files = readdirSync(DIR);
const components = files.filter((f) => f.endsWith(".svelte")).map((f) =>
  f.replace(/\.svelte$/, "")
);
const stories = new Set(
  files.filter((f) => f.endsWith(".stories.ts")).map((f) =>
    f.replace(/\.stories\.ts$/, "")
  ),
);

const missing = components.filter((c) => !stories.has(c));

if (missing.length === 0) {
  console.log(
    `story-coverage: clean — all ${components.length} components have stories.`,
  );
  process.exit(0);
}

console.error(
  "story-coverage: components without a *.stories.ts (every UI state must be declarable):\n",
);
for (const c of missing) console.error(`  ${c}.svelte  → add ${c}.stories.ts`);
process.exit(1);
