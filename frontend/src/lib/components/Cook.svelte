<script lang="ts">
  import Alert from "./Alert.svelte";
  import Notice from "./Notice.svelte";
  import type {
    CookRecipe,
    CookStatus,
    StructuredMeasure,
    StructuredStep,
  } from "$lib/types";
  import { formatAmount } from "$lib/measure";
  import { cookStages, formatClock, type StepTimer } from "$lib/steps";

  /**
   * `cook` (#36): the picked recipe in full, to follow while cooking.
   *
   * The method is the model's step reading (#74/#75/#76), rendered as a graph, not a
   * newline split: the ingredients (item + amount), a **prep** lane (mise en place),
   * and the **method** as stages where a stage holding more than one step runs in
   * parallel. Timed steps carry a timer.
   *
   * Presentational: the page owns the query and the live timer machinery (ticking,
   * alerts, persistence) and passes `timers` per step id, so every state — idle,
   * running, done — is a deterministic story rather than a live clock.
   */
  interface Props {
    status: CookStatus;
    /** The picked recipe in full, or `null` if no pick has decided yet. */
    recipe?: CookRecipe | null;
    error?: string;
    /** Live timer state per step id (seconds left + whether it fired); absent = idle. */
    timers?: Record<number, StepTimer>;
    onStartTimer?: (id: number) => void;
    onDismissTimer?: (id: number) => void;
  }

  let {
    status,
    recipe,
    error,
    timers = {},
    onStartTimer,
    onDismissTimer,
  }: Props = $props();

  /** The amount reference for one ingredient — "5", "¼ cup"; empty when unmeasured. */
  function amountOf(ing: StructuredMeasure): string {
    return formatAmount(ing.amount);
  }

  // The prep lane (mise en place) and the cooking stages, off the step DAG.
  const prep = $derived(recipe ? recipe.steps.filter((s) => s.kind === "prep") : []);
  const stages = $derived(recipe ? cookStages(recipe.steps) : []);
</script>

<div class="pt-6">
  <header class="mb-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-paprika-500" aria-hidden="true"></span>
      Cook
    </p>
  </header>

  {#if status === "error"}
    <Alert>
      <p class="font-display text-stone-900">Couldn't load the recipe.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the corpus."}
      </p>
    </Alert>
  {:else if status === "pending"}
    <div class="rounded-card mb-5 aspect-video w-full bg-stone-100" aria-hidden="true"></div>
    <div class="rounded-pill h-6 w-56 bg-stone-100" aria-hidden="true"></div>
  {:else if !recipe}
    <Notice>
      <p class="font-display text-stone-900">Nothing to cook yet.</p>
      <p class="mt-1 text-sm text-stone-600">
        Pick something first — once the group agrees on a recipe, the method shows
        up here.
      </p>
    </Notice>
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
          <li class="rounded-pill border border-stone-200 bg-cream-100 px-3 py-1 text-sm text-stone-600">
            {ing.item}{#if amountOf(ing)}<span class="text-stone-400">
                · {amountOf(ing)}</span
              >{/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if recipe.steps.length === 0}
      <p class="mt-8 text-stone-500">This recipe's method hasn't been read yet.</p>
    {:else}
      {#if prep.length}
        <!-- Prep — mise en place, done ahead and in parallel. -->
        <h2 class="font-display mt-8 mb-4 flex items-center gap-2 text-stone-600">
          <span class="size-2 rounded-full bg-paprika-500" aria-hidden="true"></span>
          Prep
        </h2>
        <ul class="flex flex-col gap-3">
          {#each prep as step (step.id)}
            <li class="flex flex-wrap items-center gap-3">
              <span class="font-display flex-1 text-stone-900">{step.text}</span>
              {@render timer(step)}
            </li>
          {/each}
        </ul>
      {/if}

      <!-- The method — the emphasis of `cook`, as stages; a stage of more than one
           step runs in parallel. -->
      <h2 class="font-display mt-8 mb-4 flex items-center gap-2 text-stone-600">
        <span class="size-2 rounded-full bg-paprika-500" aria-hidden="true"></span>
        Method
      </h2>
      <ol class="flex flex-col gap-5">
        {#each stages as stage, i (stage.depth)}
          <li class="flex gap-4">
            <span
              class="font-display flex size-8 flex-none items-center justify-center rounded-full border-2 border-paprika-500 text-sm font-medium text-paprika-500"
            >
              {i + 1}
            </span>
            <div class="flex-1">
              {#if stage.steps.length > 1}
                <p class="mb-2 text-sm text-paprika-500">At the same time</p>
              {/if}
              <ul class="flex flex-col gap-3">
                {#each stage.steps as step (step.id)}
                  <li class="flex flex-wrap items-center gap-3">
                    <p class="flex-1 text-lg leading-relaxed text-stone-900">
                      {step.text}
                    </p>
                    {@render timer(step)}
                  </li>
                {/each}
              </ul>
            </div>
          </li>
        {/each}
      </ol>
    {/if}
  {/if}
</div>

<!-- One step's timer: a Start control, a live countdown, or a done flag. -->
{#snippet timer(step: StructuredStep)}
  {#if step.seconds != null}
    {@const t = timers[step.id]}
    {#if t?.done}
      <span
        class="rounded-pill flex-none bg-paprika-500 px-3 py-1 text-sm font-medium text-cream-50"
      >
        Done · time's up
      </span>
      <button
        type="button"
        class="text-sm text-stone-500 underline"
        onclick={() => onDismissTimer?.(step.id)}
      >
        Dismiss
      </button>
    {:else if t}
      <span
        class="rounded-pill flex-none bg-paprika-100 px-3 py-1 text-sm font-medium text-paprika-500 tabular-nums"
      >
        {formatClock(t.remaining)}
      </span>
      <button
        type="button"
        class="text-sm text-stone-500 underline"
        onclick={() => onDismissTimer?.(step.id)}
      >
        Stop
      </button>
    {:else}
      <button
        type="button"
        class="rounded-pill flex-none border border-paprika-500 px-3 py-1 text-sm font-medium text-paprika-500"
        onclick={() => onStartTimer?.(step.id)}
      >
        Start {formatClock(step.seconds)}
      </button>
    {/if}
  {/if}
{/snippet}
