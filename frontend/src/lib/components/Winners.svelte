<script lang="ts">
  import type { WinCondition, Winner } from "$lib/types";

  /**
   * The cook-decider results (#20): the group's pick, ranked.
   *
   * Two win conditions, selectable — **plurality** (most yeses win) and
   * **consensus** (only what everyone said yes to). Both read off the same tally
   * the page maintains, so the toggle is a pure view over `candidates`;
   * presentational, so an empty-consensus session is a story, not a race.
   */
  interface Props {
    condition: WinCondition;
    /** Distinct voters — consensus means a yes from every one of them. */
    participants: number;
    /** Every voted-on recipe with its running tally. */
    candidates: Winner[];
    onCondition?: (c: WinCondition) => void;
    onBack?: () => void;
  }

  let { condition, participants, candidates, onCondition, onBack }: Props =
    $props();

  const shown = $derived.by(() => {
    const ranked = [...candidates].sort((a, b) => b.yes - a.yes || a.no - b.no);
    return condition === "consensus"
      ? ranked.filter((w) => participants > 0 && w.yes === participants && w.no === 0)
      : ranked.filter((w) => w.yes > 0);
  });

  function pill(active: boolean): string {
    return active
      ? "bg-pesto-500 text-cream-50"
      : "bg-cream-50 text-stone-600 border border-stone-200 hover:border-pesto-500";
  }
</script>

<div class="pt-6">
  <header class="mb-6 flex items-center justify-between gap-4">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"></span>
      What to cook
    </p>
    <button
      onclick={() => onBack?.()}
      class="rounded-pill font-display border border-stone-200 bg-cream-50 px-4 py-2 text-sm font-medium text-stone-900 transition-colors hover:border-pesto-500"
    >
      Keep swiping
    </button>
  </header>

  <div class="mb-5 inline-flex gap-1 rounded-pill bg-cream-100 p-1" role="group">
    <button
      onclick={() => onCondition?.("plurality")}
      class="rounded-pill font-display px-4 py-1.5 text-sm font-medium transition-colors {pill(
        condition === 'plurality',
      )}"
    >
      Most liked
    </button>
    <button
      onclick={() => onCondition?.("consensus")}
      class="rounded-pill font-display px-4 py-1.5 text-sm font-medium transition-colors {pill(
        condition === 'consensus',
      )}"
    >
      Everyone
    </button>
  </div>

  {#if shown.length === 0}
    <div class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center">
      <p class="font-display text-stone-900">
        {condition === "consensus"
          ? "No unanimous pick yet."
          : "Nothing in the running yet."}
      </p>
      <p class="mt-1 text-sm text-stone-600">
        {condition === "consensus"
          ? "Nobody's agreed on one recipe — keep swiping, or switch to most-liked."
          : "Swipe some cards and the group's favourites gather here."}
      </p>
    </div>
  {:else}
    <ol class="flex flex-col gap-3">
      {#each shown as w, i (`${w.card.source}:${w.card.id}`)}
        {@const meta = [w.card.category, w.card.area].filter(Boolean).join(" · ")}
        <li
          class="rounded-card flex items-center gap-4 border bg-cream-100 p-3 {i ===
          0
            ? 'border-pesto-500'
            : 'border-stone-200'}"
        >
          <span
            class="font-display w-6 flex-none text-center text-lg font-medium {i ===
            0
              ? 'text-pesto-500'
              : 'text-stone-400'}"
          >
            {i + 1}
          </span>
          {#if w.card.image}
            <img
              src={w.card.image}
              alt={w.card.title}
              class="rounded-card size-16 flex-none object-cover"
              loading="lazy"
            />
          {/if}
          <div class="min-w-0 flex-1">
            <p class="font-display truncate font-medium text-stone-900">
              {w.card.title}
            </p>
            {#if meta}
              <p class="truncate text-sm text-stone-500">{meta}</p>
            {/if}
          </div>
          <span
            class="rounded-pill flex-none bg-pesto-100 px-3 py-1 text-sm font-medium text-pesto-500"
          >
            {w.yes} yes
          </span>
        </li>
      {/each}
    </ol>
  {/if}
</div>
