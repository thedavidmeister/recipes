<script lang="ts">
  import type { HealthStatus, HealthStats } from "$lib/types";

  /**
   * The admin health dashboard: corpus size, enrichment progress, and recent runs.
   *
   * Presentational only — the page owns the query and hands this the state, so
   * every state (loading, error, not-admin, a live snapshot) is a Storybook story
   * rather than something you race the network to reach.
   */
  interface Props {
    status: HealthStatus;
    stats?: HealthStats;
    error?: string;
  }

  let { status, stats, error }: Props = $props();

  // Deterministic UTC formatting (unix seconds -> "YYYY-MM-DD HH:MM"), on purpose:
  // a relative "3m ago" depends on the wall clock, which would make the visual
  // regression baseline flicker on every capture. Same input, same pixels.
  function fmt(ts: number): string {
    return new Date(ts * 1000).toISOString().slice(0, 16).replace("T", " ");
  }
  function took(started: number, finished: number | null): string {
    if (finished == null) return "—";
    const s = finished - started;
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  }

  const pct = $derived(stats ? Math.round(stats.enriched_pct) : 0);

  // Run-status pill colours: completed reads as done (pesto), failed as danger
  // (paprika), anything else (running) as in-progress (honey).
  function badgeCls(runStatus: string): string {
    if (runStatus === "completed")
      return "border-pesto-500/30 bg-pesto-100 text-pesto-500";
    if (runStatus === "failed")
      return "border-paprika-500/30 bg-paprika-100 text-paprika-500";
    return "border-honey-500/30 bg-honey-100 text-stone-900";
  }
</script>

<div class="pt-6">
  <header class="mb-6 flex items-center gap-2">
    <span class="bg-pesto-500 size-2.5 rounded-full" aria-hidden="true"></span>
    <h1 class="font-display text-lg font-semibold text-stone-900">Corpus health</h1>
  </header>

  {#if status === "error"}
    <div class="rounded-card border-paprika-500/30 bg-paprika-100 border p-6">
      <p class="font-display text-stone-900">Could not load health.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the backend."}
      </p>
    </div>
  {:else if status === "forbidden"}
    <div class="rounded-card bg-cream-100 border border-stone-200 p-6">
      <p class="font-display text-stone-900">This page is for the admin.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "You are signed in, but this view is not for you."}
      </p>
    </div>
  {:else if status === "pending"}
    <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      {#each [0, 1, 2, 3] as i (i)}
        <div
          class="rounded-card h-20 animate-pulse border border-stone-200 bg-stone-100"
        ></div>
      {/each}
    </div>
  {:else if stats}
    <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      {@render tile("Recipes", stats.recipes.toLocaleString(), "", false)}
      {@render tile("Raw imports", stats.raw.toLocaleString(), "", false)}
      {@render tile("Enriched", stats.enriched.toLocaleString(), `${pct}%`, false)}
      {@render tile(
        "Running",
        stats.running.toLocaleString(),
        stats.running > 0 ? "in flight" : "idle",
        stats.running > 0,
      )}
    </div>

    <section class="rounded-card bg-cream-50 mt-4 border border-stone-200 p-5">
      <div class="mb-2 flex items-baseline justify-between gap-3">
        <h2 class="font-display text-sm font-semibold text-stone-900">
          Enrichment
        </h2>
        <p class="text-sm text-stone-500">
          {stats.enriched.toLocaleString()} of {stats.recipes.toLocaleString()} read
          · {pct}%
        </p>
      </div>
      <div class="rounded-pill h-2 overflow-hidden bg-stone-100">
        <div
          class="bg-pesto-500 rounded-pill h-full transition-[width]"
          style="width: {pct}%"
        ></div>
      </div>
      {#if stats.by_model.length > 0}
        <ul class="mt-3 flex flex-wrap gap-2">
          {#each stats.by_model as m (m.model)}
            <li
              class="rounded-pill bg-cream-100 border border-stone-200 px-2.5 py-1 text-xs text-stone-600"
            >
              <span class="font-medium text-stone-900">{m.model}</span> ·
              {m.count.toLocaleString()}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="mt-3 text-xs text-stone-500">
          No readings yet — the enrich worker has not run.
        </p>
      {/if}
    </section>

    <section class="rounded-card bg-cream-50 mt-4 border border-stone-200 p-5">
      <h2 class="font-display mb-3 text-sm font-semibold text-stone-900">
        Recent runs
      </h2>
      {#if stats.recent_runs.length === 0}
        <p class="text-sm text-stone-500">No runs recorded yet.</p>
      {:else}
        <div class="overflow-x-auto">
          <table class="w-full text-left text-sm">
            <thead>
              <tr class="text-xs text-stone-500">
                <th class="pr-3 pb-2 font-medium">#</th>
                <th class="pr-3 pb-2 font-medium">Kind</th>
                <th class="pr-3 pb-2 font-medium">Status</th>
                <th class="pr-3 pb-2 font-medium">Started (UTC)</th>
                <th class="pb-2 font-medium">Took</th>
              </tr>
            </thead>
            <tbody>
              {#each stats.recent_runs as run (run.id)}
                <tr class="border-t border-stone-100">
                  <td class="pr-3 py-1.5 text-stone-500 tabular-nums">{run.id}</td>
                  <td class="pr-3 py-1.5 text-stone-900">{run.kind}</td>
                  <td class="pr-3 py-1.5">{@render badge(run.status)}</td>
                  <td class="pr-3 py-1.5 text-stone-600 tabular-nums">
                    {fmt(run.started_at)}
                  </td>
                  <td class="py-1.5 text-stone-600 tabular-nums">
                    {took(run.started_at, run.finished_at)}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
  {/if}
</div>

{#snippet tile(label: string, value: string, sub: string, warn: boolean)}
  <div
    class="rounded-card border p-4 {warn
      ? 'border-paprika-500/40 bg-paprika-100'
      : 'bg-cream-50 border-stone-200'}"
  >
    <p class="text-xs text-stone-500">{label}</p>
    <p
      class="font-display mt-1 text-2xl font-semibold tabular-nums {warn
        ? 'text-paprika-500'
        : 'text-stone-900'}"
    >
      {value}
    </p>
    {#if sub}<p class="text-xs text-stone-500">{sub}</p>{/if}
  </div>
{/snippet}

{#snippet badge(runStatus: string)}
  <span
    class="rounded-pill border px-2 py-0.5 text-xs font-medium {badgeCls(
      runStatus,
    )}"
  >
    {runStatus}
  </span>
{/snippet}
