<script lang="ts">
  import type { Recipe } from "$lib/types";

  let { recipe }: { recipe: Recipe } = $props();

  const meta = $derived(
    [recipe.category, recipe.area].filter(Boolean).join(" · "),
  );
</script>

<article class="overflow-hidden rounded-card border border-stone-200 bg-cream-100">
  {#if recipe.image}
    <img
      src={recipe.image}
      alt={recipe.title}
      class="aspect-video w-full object-cover"
      loading="lazy"
    />
  {/if}
  <div class="p-4">
    <h2 class="font-display text-lg font-medium text-stone-900">{recipe.title}</h2>
    {#if meta}
      <p class="mt-1 text-sm text-stone-500">{meta}</p>
    {/if}
  </div>
</article>
