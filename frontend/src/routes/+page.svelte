<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { searchThemealdb } from "$lib/sources";

  let term = $state("");
  let submitted = $state("");

  const results = createQuery(() => ({
    queryKey: ["themealdb", submitted],
    queryFn: () => searchThemealdb(submitted),
    enabled: submitted.length > 0,
  }));

  function search(event: SubmitEvent) {
    event.preventDefault();
    submitted = term.trim();
  }
</script>

<main class="mx-auto max-w-5xl px-4 py-10">
  <h1 class="text-3xl font-bold tracking-tight">recipes</h1>
  <p class="mt-1 text-neutral-500">Search a public recipe database.</p>

  <form onsubmit={search} class="mt-6 flex gap-2">
    <input
      bind:value={term}
      placeholder="chicken, pasta, curry…"
      aria-label="Search recipes"
      class="flex-1 rounded-lg border border-neutral-300 px-3 py-2 outline-none focus:border-neutral-900"
    />
    <button
      class="rounded-lg bg-neutral-900 px-4 py-2 font-medium text-white hover:bg-neutral-700"
    >
      Search
    </button>
  </form>

  {#if results.isPending && submitted}
    <p class="mt-8 text-neutral-500">Searching…</p>
  {:else if results.isError}
    <p class="mt-8 text-red-600">Something went wrong. Try again.</p>
  {:else if results.data}
    {#if results.data.length === 0}
      <p class="mt-8 text-neutral-500">No recipes found for “{submitted}”.</p>
    {:else}
      <div class="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {#each results.data as recipe (recipe.source + recipe.id)}
          <article
            class="overflow-hidden rounded-xl border border-neutral-200 bg-white"
          >
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
              <p class="mt-1 text-sm text-neutral-500">
                {[recipe.category, recipe.area].filter(Boolean).join(" · ")}
              </p>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  {/if}
</main>
