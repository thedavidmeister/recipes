<script lang="ts">
  import type { BuyRecipe, BuyStatus } from "$lib/types";

  /**
   * `buy` (#36): the shopping list for the pick's consensus recipe.
   *
   * The step after `pick` in the meal arc — what the group agreed on, and what it
   * needs. Presentational only: the page owns the query (the recipe is read from
   * the pick's stashed decision), this renders. Every state is a Storybook story.
   */
  interface Props {
    status: BuyStatus;
    /** The consensus recipe + its ingredients, or `null` if no pick has decided. */
    recipe?: BuyRecipe | null;
    error?: string;
  }

  let { status, recipe, error }: Props = $props();
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
          class="rounded-card flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3"
        >
          <span class="rounded-pill h-4 w-40 bg-stone-100"></span>
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
    <ul class="flex flex-col gap-2">
      {#each recipe.ingredients as ing, i (i)}
        <li
          class="rounded-card flex items-center justify-between gap-4 border border-stone-200 bg-cream-100 px-4 py-3"
        >
          <span class="font-display text-stone-900">{ing.name}</span>
          {#if ing.measure}
            <span
              class="rounded-pill flex-none bg-plum-100 px-3 py-1 text-sm text-stone-600"
            >
              {ing.measure}
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
