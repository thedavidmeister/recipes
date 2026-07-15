<script lang="ts">
  import type { ImportResult } from "$lib/types";
  import RecipeCard from "./RecipeCard.svelte";

  let {
    result,
    pending = false,
  }: { result?: ImportResult | null; pending?: boolean } = $props();
</script>

{#if pending}
  <p class="mt-6 text-neutral-500">Fetching and parsing…</p>
{:else if result}
  {#if result.kind === "saved"}
    <p class="mt-6 text-green-700">Saved “{result.recipe.title}” to the corpus.</p>
    <div class="mt-3 max-w-sm">
      <RecipeCard recipe={result.recipe} />
    </div>
  {:else if result.kind === "incomplete"}
    <p class="mt-6 text-amber-700">
      Found “{result.recipe.title}”, but it has no ingredients or instructions —
      not saved.
    </p>
    <div class="mt-3 max-w-sm">
      <RecipeCard recipe={result.recipe} />
    </div>
  {:else if result.kind === "no-recipe"}
    <p class="mt-6 text-neutral-500">
      No recipe found on that page — it publishes no schema.org/Recipe data.
    </p>
  {:else if result.kind === "invalid-url"}
    <p class="mt-6 text-red-600">{result.message}</p>
  {:else if result.kind === "fetch-failed"}
    <p class="mt-6 text-red-600">Couldn’t fetch that page. {result.message}</p>
  {:else if result.kind === "save-failed"}
    <p class="mt-6 text-red-600">
      Parsed “{result.recipe.title}” but saving failed. {result.message}
    </p>
  {/if}
{/if}
