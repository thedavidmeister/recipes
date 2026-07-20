<script lang="ts">
  import type { CookRecipe, CookStatus, StructuredMeasure } from "$lib/types";
  import { formatAmount } from "$lib/measure";

  /**
   * `cook` (#36): the picked recipe in full, to follow while cooking.
   *
   * The step after `buy`. Three parts from the structured reading (#11): the
   * ingredients (`item` + measured `amount`), the **prep** — each ingredient's
   * `preparation` ("thinly sliced"), a process, gathered into its own mise-en-place
   * section rather than buried in the ingredient line — and the **method**, the big
   * numbered steps. Presentational only: the page owns the query, this renders.
   */
  interface Props {
    status: CookStatus;
    /** The picked recipe in full, or `null` if no pick has decided yet. */
    recipe?: CookRecipe | null;
    error?: string;
  }

  let { status, recipe, error }: Props = $props();

  /** The amount reference for one ingredient — "5", "¼ cup"; empty when unmeasured. */
  function amountOf(ing: StructuredMeasure): string {
    return formatAmount(ing.amount);
  }

  // Mise en place: the ingredients that need a preparation before cooking. The
  // preparation is a process, so it earns its own section instead of trailing the
  // ingredient — the split `buy`/`cook` are built on.
  const prep = $derived(
    recipe
      ? recipe.ingredients.filter(
          (i): i is StructuredMeasure & { preparation: string } =>
            typeof i.preparation === "string" && i.preparation.trim() !== "",
        )
      : [],
  );

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
            {ing.item}{#if amountOf(ing)}<span class="text-stone-400">
                · {amountOf(ing)}</span
              >{/if}
          </li>
        {/each}
      </ul>
    {/if}

    <!-- Prep — each ingredient's process, the mise en place before the method. -->
    {#if prep.length}
      <h2 class="font-display mt-8 mb-4 flex items-center gap-2 text-stone-600">
        <span class="size-2 rounded-full bg-paprika-500" aria-hidden="true"></span>
        Prep
      </h2>
      <ul class="flex flex-col gap-2">
        {#each prep as ing, i (i)}
          <li class="flex items-baseline gap-2">
            <span class="font-display text-stone-900">{ing.item}</span>
            <span class="text-stone-500">— {ing.preparation}</span>
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
