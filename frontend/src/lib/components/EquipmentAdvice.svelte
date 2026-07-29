<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import type { EquipmentAdvice, KitchensStatus } from "$lib/types";

  /**
   * What a kitchen should add next (#83), under the equipment list it is advice about.
   *
   * It lives here rather than on the kitchen page because the kitchen page is about
   * being in the kitchen (#119) and this is about changing it — and because the one
   * thing to do about a recommendation is add the item, which is the field directly
   * above (#117 put equipment under settings).
   *
   * Every number on the page is a running total: line three counts what you could make
   * holding lines one, two and three. That has to be said in the copy, because read as
   * a per-item figure it would overclaim on every line but the first. The alternative —
   * a per-item "adds 12" — is only honest for the first line, since past that it
   * silently assumes the ones above were bought too.
   */
  interface Props {
    status: KitchensStatus;
    advice?: EquipmentAdvice | null;
    error?: string;
  }

  let { status, advice, error }: Props = $props();

  /** Sentence case for display only; the stored key is the normalised lowercase name. */
  const display = (s: string) =>
    s ? s.charAt(0).toUpperCase() + s.slice(1) : s;

  const recipes = (n: number) => `${n} ${n === 1 ? "recipe" : "recipes"}`;

  /**
   * Whether the first line stands on its own. When it does not, this kitchen is
   * several items from the nearest recipe — the ordinary state of a kitchen with
   * little recorded, and the reason the list is a list rather than one suggestion.
   */
  const immediate = $derived((advice?.additions[0]?.unlocks ?? 0) > 0);
  const total = $derived(advice?.additions.at(-1)?.unlocks ?? 0);
</script>

<section class="mt-8 border-t border-stone-200 pt-6">
  <h2 class="font-display text-lg font-medium text-stone-900">
    What to add next
  </h2>

  {#if status === "error"}
    <p class="mt-2 text-sm text-stone-600">
      {error ?? "Couldn't work out what to add."}
    </p>
  {:else if status === "pending" || !advice}
    <div class="mt-3"><Skeleton /></div>
  {:else if advice.read === 0}
    <p class="mt-2 text-sm text-stone-600">
      Nothing to suggest yet — no recipe here says what equipment it needs. So
      there is nothing to count, and nothing is being left out of a meal for
      want of a tool.
    </p>
  {:else if advice.additions.length === 0}
    <p class="mt-2 text-sm text-stone-600">
      Nothing to add — every recipe is already in reach of this kitchen.
    </p>
  {:else}
    <p class="mt-2 text-sm text-stone-600">
      {#if immediate}
        {display(advice.additions[0].item)} on its own puts {recipes(
          advice.additions[0].unlocks,
        )} in reach.
      {:else}
        Nothing here gets you a recipe on its own — this kitchen is several
        items from the nearest one. Together they do.
      {/if}
    </p>

    <ol class="mt-4 flex flex-col gap-2">
      {#each advice.additions as addition, i (addition.item)}
        <li
          class="rounded-card bg-cream-100 flex items-baseline justify-between gap-3 border border-stone-200 px-4 py-3"
        >
          <span class="text-stone-900">
            <span class="mr-2 text-sm text-stone-400">{i + 1}</span>{display(
              addition.item,
            )}
          </span>
          <span class="flex-none text-sm text-stone-500"
            >{recipes(addition.unlocks)}</span
          >
        </li>
      {/each}
    </ol>

    <p class="mt-3 text-sm text-stone-500">
      {#if advice.additions.length > 1}
        Each line counts what you could make with it <em
          >and everything above it</em
        >, so the bottom line is the lot — {recipes(total)}.
      {/if}
      This kitchen can make {recipes(advice.makeable)} today.
    </p>
  {/if}
</section>
