<script lang="ts">
  import type { WalkStatus, WalkStop } from "$lib/types";

  /**
   * The `pick` walk (#47), rendered as a **journey**: a vertical run of stops,
   * each a recipe, threaded by the ingredient crossed to reach it ("via miso").
   * The thread is the narrative — it is what makes a walk read as wandering with a
   * reason rather than a shuffle; the first stop has none, being where the wander
   * began.
   *
   * Presentational only. The page owns the query and hands this the state, so
   * every state — loading, error, an empty corpus, a full walk — is a Storybook
   * story rather than something you race the network to reach. The rail down the
   * left echoes the section nav's "you are on a line" language, in pesto because
   * this is the pick section.
   */
  interface Props {
    status: WalkStatus;
    stops?: WalkStop[];
    error?: string;
    /** A walk is in flight (the first load, or a "walk again"). */
    busy?: boolean;
    /** Re-roll the walk. The page wires this to a refetch. */
    onAgain?: () => void;
  }

  let { status, stops = [], error, busy = false, onAgain }: Props = $props();
</script>

<div class="pt-6">
  <header class="mb-6 flex items-center justify-between gap-4">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"></span>
      A wander through the pantry
    </p>
    <button
      onclick={() => onAgain?.()}
      disabled={busy}
      class="rounded-pill font-display inline-flex items-center gap-2 border border-stone-200 bg-cream-50 px-4 py-2 text-sm font-medium text-stone-900 transition-colors hover:border-pesto-500 disabled:opacity-50"
    >
      <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"></span>
      {busy ? "Wandering…" : "Walk again"}
    </button>
  </header>

  {#if status === "error"}
    <div class="rounded-card border border-paprika-500/30 bg-paprika-100 p-6">
      <p class="font-display text-stone-900">The walk stumbled.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the corpus."}
      </p>
      <button
        onclick={() => onAgain?.()}
        class="mt-4 text-sm font-medium text-paprika-500 underline hover:text-stone-900"
      >
        Try again
      </button>
    </div>
  {:else if status === "pending"}
    <!-- A quiet skeleton of the journey to come: rail, dots centred on
         stop-shaped blanks. Static, not pulsing, so it renders identically every
         time. -->
    <ol aria-hidden="true">
      {#each Array(4) as _, i (i)}
        <li class="flex gap-4">
          <div class="flex w-4 flex-none flex-col items-center">
            <span
              class="w-0.5 flex-1 {i > 0 ? 'bg-stone-200' : 'bg-transparent'}"
            ></span>
            <span
              class="size-4 flex-none rounded-full border-2 border-stone-200 bg-cream-50"
            ></span>
            <span
              class="w-0.5 flex-1 {i < 3 ? 'bg-stone-200' : 'bg-transparent'}"
            ></span>
          </div>
          <div
            class="rounded-card my-3 flex flex-1 gap-3 border border-stone-200 bg-cream-100"
          >
            <div class="size-20 flex-none rounded-l-card bg-stone-100"></div>
            <div class="flex flex-col justify-center gap-2 py-2">
              <div class="rounded-pill h-4 w-40 bg-stone-100"></div>
              <div class="rounded-pill h-3 w-24 bg-stone-100"></div>
            </div>
          </div>
        </li>
      {/each}
    </ol>
  {:else if stops.length === 0}
    <div class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center">
      <p class="font-display text-stone-900">Nothing to wander yet.</p>
      <p class="mt-1 text-sm text-stone-600">
        The corpus is empty — once recipes are ingested, a walk starts here.
      </p>
    </div>
  {:else}
    <!--
      Each stop is a recipe on the rail — a dot centred on its card, exactly the
      "you are on a line" language of the section nav. The ingredient crossed to
      reach a stop rides the connector *above* it ("via miso"), so the thread reads
      as the hop between two recipes rather than a label on one. The first stop has
      no connector; it is where the wander began.
    -->
    <ol>
      {#each stops as stop, i (stop.recipe.source + stop.recipe.id + i)}
        {@const meta = [stop.recipe.category, stop.recipe.area]
          .filter(Boolean)
          .join(" · ")}
        <li>
          {#if stop.via}
            <div class="flex gap-4">
              <div class="flex w-4 flex-none justify-center">
                <span class="h-7 w-0.5 bg-pesto-500/40" aria-hidden="true"></span>
              </div>
              <p class="py-1 text-sm text-stone-500">
                via <span
                  class="rounded-pill bg-pesto-100 px-2 py-0.5 font-medium text-pesto-500"
                  >{stop.via}</span
                >
              </p>
            </div>
          {/if}
          <div class="flex gap-4">
            <!-- The rail cell: a line up to the dot, the dot centred on the card,
                 a line down to the next connector. The ends fade to transparent so
                 nothing dangles past the first or last stop. -->
            <div class="flex w-4 flex-none flex-col items-center">
              <span
                class="w-0.5 flex-1 {i > 0 ? 'bg-pesto-500/40' : 'bg-transparent'}"
                aria-hidden="true"
              ></span>
              <span
                class="size-4 flex-none rounded-full border-2 border-pesto-500 bg-cream-50"
                aria-hidden="true"
              ></span>
              <span
                class="w-0.5 flex-1 {i < stops.length - 1
                  ? 'bg-pesto-500/40'
                  : 'bg-transparent'}"
                aria-hidden="true"
              ></span>
            </div>

            <article
              class="rounded-card flex min-w-0 flex-1 gap-3 overflow-hidden border border-stone-200 bg-cream-100"
            >
              {#if stop.recipe.image}
                <img
                  src={stop.recipe.image}
                  alt={stop.recipe.title}
                  class="size-20 flex-none object-cover"
                  loading="lazy"
                />
              {/if}
              <div class="min-w-0 self-center py-2 pr-3">
                <h2
                  class="font-display truncate text-base font-medium text-stone-900"
                >
                  {stop.recipe.title}
                </h2>
                {#if meta}
                  <p class="mt-0.5 truncate text-sm text-stone-500">{meta}</p>
                {/if}
              </div>
            </article>
          </div>
        </li>
      {/each}
    </ol>
  {/if}
</div>
