<script lang="ts">
  import type { CookRecipe, CookStatus } from "$lib/types";

  /**
   * `cook` (#36): the picked recipe in full, to follow while cooking.
   *
   * The step after `buy` — the **method** is the star (big, numbered steps), with
   * the ingredients riding along as a compact reference. Presentational only: the
   * page owns the query (the recipe is the pick's stashed decision), this renders.
   */
  interface Props {
    status: CookStatus;
    /** The picked recipe in full, or `null` if no pick has decided yet. */
    recipe?: CookRecipe | null;
    error?: string;
  }

  let { status, recipe, error }: Props = $props();

  // Split the instructions into steps — one per non-blank line.
  const steps = $derived(
    recipe
      ? recipe.instructions
          .split(/\r?\n+/)
          .map((s) => s.trim())
          .filter(Boolean)
      : [],
  );
</script>

<div class="pt-6">
  <header class="mb-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-paprika-500" aria-hidden="true"
      ></span>
      Cook
    </p>
  </header>

  {#if status === "error"}
    <div class="rounded-card border border-paprika-500/30 bg-paprika-100 p-6">
      <p class="font-display text-stone-900">Couldn't load the recipe.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the corpus."}
      </p>
    </div>
  {:else if status === "pending"}
    <div class="rounded-card mb-5 aspect-video w-full bg-stone-100" aria-hidden="true"></div>
    <div class="rounded-pill h-6 w-56 bg-stone-100" aria-hidden="true"></div>
  {:else if !recipe}
    <div
      class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
    >
      <p class="font-display text-stone-900">Nothing to cook yet.</p>
      <p class="mt-1 text-sm text-stone-600">
        Pick something first — once the group agrees on a recipe, the method shows
        up here.
      </p>
    </div>
  {:else}
    {#if recipe.image}
      <img
        src={recipe.image}
        alt={recipe.title}
        class="rounded-card mb-5 aspect-video w-full object-cover"
        loading="lazy"
      />
    {/if}
    <h1 class="font-display text-2xl font-medium text-stone-900">
      {recipe.title}
    </h1>

    {#if recipe.ingredients.length}
      <ul class="mt-4 flex flex-wrap gap-2">
        {#each recipe.ingredients as ing, i (i)}
          <li
            class="rounded-pill border border-stone-200 bg-cream-100 px-3 py-1 text-sm text-stone-600"
          >
            {ing.name}{#if ing.measure}<span class="text-stone-400">
                · {ing.measure}</span
              >{/if}
          </li>
        {/each}
      </ul>
    {/if}

    <!-- The method — the emphasis of `cook`. -->
    <h2 class="font-display mt-8 mb-4 flex items-center gap-2 text-stone-600">
      <span class="size-2 rounded-full bg-paprika-500" aria-hidden="true"></span>
      Method
    </h2>
    {#if steps.length}
      <ol class="flex flex-col gap-5">
        {#each steps as step, i (i)}
          <li class="flex gap-4">
            <span
              class="font-display flex size-8 flex-none items-center justify-center rounded-full border-2 border-paprika-500 text-sm font-medium text-paprika-500"
            >
              {i + 1}
            </span>
            <p class="flex-1 text-lg leading-relaxed text-stone-900">{step}</p>
          </li>
        {/each}
      </ol>
    {:else}
      <p class="text-stone-500">No method listed for this recipe.</p>
    {/if}
  {/if}
</div>
