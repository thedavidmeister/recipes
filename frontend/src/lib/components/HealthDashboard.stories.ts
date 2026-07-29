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

/**
 * The gap between an ingest and the enrich cron: new recipes are in, unread. Every
 * ingest opens this window, so the part-read bar is a state the corpus really passes
 * through.
 *
 * The numbers are chosen rather than read, and there is no reading to take — every
 * enrichment stage is at 790 of 790 today. 745 of 790 is the shape the gap takes,
 * and both ends of it are real: 745 is what the corpus held before the growth that
 * left the old fixture stale, 790 is what it holds now.
 */
export const PartlyRead: Story = {
  args: {
    status: "ready",
    stats: healthStats({
      enriched: 745,
      enriched_pct: (745 / 790) * 100,
      by_model: [{ model: "claude-sonnet-5", count: 745 }],
    }),
  },
};

/**
 * A run that failed — the pill goes paprika.
 *
 * Reachable since #174. `runs::finish` now takes the outcome the run actually
 * reached, so a derive that blew up inside an ingest, or a push that errored partway
 * through a batch, closes itself `failed`; before that, `finish` was only ever called
 * with `COMPLETED` and no row in the corpus had carried this status in the database's
 * lifetime (223 `completed`, 22 `running`, 0 `failed`). A run that *dies* still
 * leaves itself `running`, which is what the `Ready` snapshot above shows 22 of —
 * that is a different fact and it keeps its own word.
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

/**
 * A run that did part of its job — the pill goes honey.
 *
 * The state #174 added a word for, and the one a scheduled ingest reaches most often:
 * the sync could not reach every source (each dropped URL is named in the run's
 * report), and it derived what it did have. Calling that `failed` would have pinned
 * the column to one value again — sources 502 scrapers as a matter of routine — and
 * calling it `completed` is what made this table answer "yes" for every run.
 */
export const RunPartial: Story = {
  args: {
    status: "ready",
    stats: healthStats({
      recent_runs: [
        {
          id: 246,
          kind: "ingest",
          status: "partial",
          started_at: 1_785_214_900,
          finished_at: 1_785_214_961,
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
  args: { status: "error", error: "Couldn't load health (502)." },
};
