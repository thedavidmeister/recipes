<script lang="ts">
  import type { Recipe } from "$lib/types";

  let { recipe }: { recipe: Recipe } = $props();

  const meta = $derived(
    [recipe.category, recipe.area].filter(Boolean).join(" · "),
  );
</script>

<article class="overflow-hidden rounded-xl border border-neutral-200 bg-white">
  {#if recipe.image}
    <img
      src={recipe.image}
      alt={recipe.title}
      class="aspect-video w-full object-cover"
      loading="lazy"
    />
  {/if}
  <div class="p-4">
    <h2 class="font-semibold">{recipe.title}</h2>
    {#if meta}
      <p class="mt-1 text-sm text-neutral-500">{meta}</p>
    {/if}
  </div>
</article>
