import type { Meta, StoryObj } from "@storybook/sveltekit";
import HealthDashboard from "./HealthDashboard.svelte";
import { healthStats } from "$lib/fixtures";

const meta = {
  title: "recipes/HealthDashboard",
  component: HealthDashboard,
} satisfies Meta<typeof HealthDashboard>;
export default meta;

type Story = StoryObj<typeof meta>;

/** A live snapshot: a part-read corpus, a failed run in the history. */
export const Ready: Story = {
  args: { status: "ready", stats: healthStats() },
};

/** Fresh corpus — ingested but nothing enriched, no runs yet (today's real state). */
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

/** A run is in flight — the "Running" tile turns red and the run reads in-progress. */
export const RunInFlight: Story = {
  args: {
    status: "ready",
    stats: healthStats({
      running: 1,
      recent_runs: [
        {
          id: 28,
          kind: "enrich",
          status: "running",
          started_at: 1_752_849_700,
          finished_at: null,
        },
        ...healthStats().recent_runs,
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
