import type { Meta, StoryObj } from "@storybook/sveltekit";
import HealthDashboard from "./HealthDashboard.svelte";
import { healthStats } from "$lib/fixtures";

const meta = {
  title: "recipes/HealthDashboard",
  component: HealthDashboard,
} satisfies Meta<typeof HealthDashboard>;
export default meta;

type Story = StoryObj<typeof meta>;

/**
 * The corpus's real reading, taken 2026-07-28 (#157): 790 recipes, all of them read,
 * and the five most recent runs.
 *
 * The red "Running" tile is the dashboard doing its job, not a busy pipeline — 22
 * runs are still `running`, including the two most recent ingests, which `runs.rs`
 * calls the died-mid-flight signal. The fixture it replaces claimed a part-read
 * corpus of 745 and a failed run; the first had simply gone stale, and the second is
 * below.
 */
export const Ready: Story = {
  args: { status: "ready", stats: healthStats() },
};

/** Fresh corpus — ingested but nothing enriched, no runs yet. What a new deployment
 * shows on its first morning; the live corpus was last in it long ago. */
export const Empty: Story = {
  args: {
    status: "ready",
    stats: healthStats({
      enriched: 0,
      enriched_pct: 0,
      by_model: [],
      recent_runs: [],
      running: 0,
    }),
  },
};

/** The gap between an ingest and the enrich cron: new recipes are in, unread. Every
 * ingest opens this window, so the part-read bar is a state the corpus really passes
 * through — the numbers are a moment in it, not a reading of one. */
export const PartlyRead: Story = {
  args: {
    status: "ready",
    stats: healthStats({
      enriched: 712,
      enriched_pct: (712 / 790) * 100,
      by_model: [{ model: "claude-sonnet-5", count: 712 }],
    }),
  },
};

/**
 * A run that failed — the pill goes paprika.
 *
 * Depicted, and worth flagging as such: `runs::FAILED` exists in the backend but no
 * caller passes it (`finish` is only ever called with `COMPLETED`), so no row in the
 * corpus has ever carried this status. A run that dies leaves itself `running`
 * instead, which is what the `Ready` snapshot above shows 22 of.
 */
export const RunFailed: Story = {
  args: {
    status: "ready",
    stats: healthStats({
      recent_runs: [
        {
          id: 246,
          kind: "enrich_steps",
          status: "failed",
          started_at: 1_785_214_900,
          finished_at: 1_785_214_915,
        },
        ...healthStats().recent_runs.slice(0, 4),
      ],
    }),
  },
};

/** Loading the snapshot. */
export const Pending: Story = { args: { status: "pending" } };

/** Signed in, but not the admin. */
export const Forbidden: Story = {
  args: { status: "forbidden", error: "This page is for the admin." },
};

/** The endpoint could not be reached. */
export const Error: Story = {
  args: { status: "error", error: "could not load health (502)" },
};
