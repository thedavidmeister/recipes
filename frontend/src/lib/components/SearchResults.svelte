<script lang="ts">
  import type { Recipe, SearchStatus } from "$lib/types";
  import RecipeCard from "./RecipeCard.svelte";

  let {
    status,
    recipes = [],
    term = "",
  }: { status: SearchStatus; recipes?: Recipe[]; term?: string } = $props();
</script>

{#if status === "pending"}
  <p class="mt-8 text-stone-500">Searching…</p>
{:else if status === "error"}
  <p class="mt-8 text-tomato-500">Something went wrong. Try again.</p>
{:else if status === "ready"}
  {#if recipes.length === 0}
    <p class="mt-8 text-stone-500">No recipes found for “{term}”.</p>
  {:else}
    <div class="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {#each recipes as recipe (recipe.source + recipe.id)}
        <RecipeCard {recipe} />
      {/each}
    </div>
  {/if}
{/if}
