<script lang="ts">
  import type { BuyRecipe, BuyStatus } from "$lib/types";

  /**
   * `buy` (#36): the shopping **checklist** for the pick's consensus recipe.
   *
   * The step after `pick` — what the group agreed on, and what it needs. Each row
   * ticks off as you shop; the page owns the ticked state (persisted per recipe,
   * so it survives a reload mid-shop) and this renders. Every state is a story.
   */
  interface Props {
    status: BuyStatus;
    /** The consensus recipe + its ingredients, or `null` if no pick has decided. */
    recipe?: BuyRecipe | null;
    error?: string;
    /** Which ingredient indices are ticked off (in the basket). */
    checked?: Record<number, boolean>;
    onToggle?: (index: number) => void;
  }

  let { status, recipe, error, checked = {}, onToggle }: Props = $props();

  const ticked = $derived(
    recipe ? recipe.ingredients.filter((_, i) => checked[i]).length : 0,
  );
</script>

<div class="pt-6">
  <header class="mb-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-plum-500" aria-hidden="true"></span>
      Buy
    </p>
    {#if status === "ready" && recipe}
      <p class="mt-1 text-sm text-stone-500">
        Everything you need for {recipe.title}.
      </p>
    {/if}
  </header>

  {#if status === "error"}
    <div class="rounded-card border border-paprika-500/30 bg-paprika-100 p-6">
      <p class="font-display text-stone-900">Couldn't load the list.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the corpus."}
      </p>
    </div>
  {:else if status === "pending"}
    <ul class="flex flex-col gap-2" aria-hidden="true">
      {#each Array(8) as _, i (i)}
        <li
          class="rounded-card flex items-center gap-3 border border-stone-200 bg-cream-100 px-4 py-3"
        >
          <span class="size-5 flex-none rounded-md bg-stone-100"></span>
          <span class="rounded-pill h-4 flex-1 bg-stone-100"></span>
          <span class="rounded-pill h-4 w-16 bg-stone-100"></span>
        </li>
      {/each}
    </ul>
  {:else if !recipe}
    <div
      class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
    >
      <p class="font-display text-stone-900">Nothing to buy yet.</p>
      <p class="mt-1 text-sm text-stone-600">
        Pick something first — once the group agrees on a recipe, its ingredients
        land here.
      </p>
    </div>
  {:else if recipe.ingredients.length === 0}
    <div
      class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
    >
      <p class="font-display text-stone-900">{recipe.title}</p>
      <p class="mt-1 text-sm text-stone-600">No ingredients listed for it yet.</p>
    </div>
  {:else}
    <p class="mb-3 text-sm text-stone-500">
      {ticked} of {recipe.ingredients.length} in the basket
    </p>
    <ul class="flex flex-col gap-2">
      {#each recipe.ingredients as ing, i (i)}
        <li>
          <label
            class="rounded-card flex cursor-pointer items-center gap-3 border border-stone-200 bg-cream-100 px-4 py-3"
          >
            <input
              type="checkbox"
              checked={!!checked[i]}
              onchange={() => onToggle?.(i)}
              class="size-5 flex-none accent-plum-500"
            />
            <span
              class="font-display flex-1 {checked[i]
                ? 'text-stone-400 line-through'
                : 'text-stone-900'}"
            >
              {ing.name}
            </span>
            {#if ing.measure}
              <span
                class="rounded-pill flex-none bg-plum-100 px-3 py-1 text-sm {checked[
                  i
                ]
                  ? 'text-stone-400'
                  : 'text-stone-600'}"
              >
                {ing.measure}
              </span>
            {/if}
          </label>
        </li>
      {/each}
    </ul>
  {/if}
</div>
