<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import {
    searchThemealdb,
    listCategories,
    browseCategory,
  } from "$lib/sources";
  import { saveRecipes, importFromUrl } from "$lib/backend";
  import type { ImportResult, Recipe, SearchStatus } from "$lib/types";
  import SearchResults from "$lib/components/SearchResults.svelte";
  import CategoryPicker from "$lib/components/CategoryPicker.svelte";
  import ImportResultView from "$lib/components/ImportResult.svelte";

  let term = $state("");
  let submitted = $state("");
  let category = $state("");
  let importUrl = $state("");
  let importing = $state(false);
  let imported = $state<ImportResult | null>(null);

  // Searching and browsing are one result list: the last action wins.
  const results = createQuery(() => ({
    queryKey: ["themealdb", "search", submitted],
    queryFn: () => searchThemealdb(submitted),
    enabled: submitted.length > 0,
  }));

  const browsed = createQuery(() => ({
    queryKey: ["themealdb", "category", category],
    queryFn: () => browseCategory(category),
    enabled: category.length > 0,
  }));

  const categories = createQuery(() => ({
    queryKey: ["themealdb", "categories"],
    queryFn: listCategories,
  }));

  const active = $derived(category ? browsed : results);
  const label = $derived(category || submitted);

  const status = $derived<SearchStatus>(
    !label
      ? "idle"
      : active.isError
        ? "error"
        : active.isPending
          ? "pending"
          : "ready",
  );

  // Persisting the corpus is a side effect of finding recipes, never a gate on
  // rendering them: saveRecipes skips partials (category browse returns header
  // fields only) and swallows individual failures.
  $effect(() => {
    const found: Recipe[] | undefined = active.data;
    if (found?.length) void saveRecipes(found).catch(() => {});
  });

  function search(event: SubmitEvent) {
    event.preventDefault();
    category = "";
    submitted = term.trim();
  }

  function browse(next: string) {
    submitted = "";
    term = "";
    category = next;
  }

  // importFromUrl returns every outcome as a value, so nothing here throws.
  async function importUrlSubmit(event: SubmitEvent) {
    event.preventDefault();
    importing = true;
    imported = null;
    try {
      imported = await importFromUrl(importUrl);
    } finally {
      importing = false;
    }
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
      class="flex-1 rounded-lg border border-neutral-300 px-3 py-2 outline-hidden focus:border-neutral-900"
    />
    <button
      class="rounded-lg bg-neutral-900 px-4 py-2 font-medium text-white hover:bg-neutral-700"
    >
      Search
    </button>
  </form>

  <div class="mt-3 max-w-xs">
    <CategoryPicker
      categories={categories.data ?? []}
      bind:value={category}
      onSelect={browse}
    />
  </div>

  <SearchResults {status} recipes={active.data ?? []} term={label} />

  <section class="mt-12 border-t border-neutral-200 pt-8">
    <h2 class="text-xl font-semibold tracking-tight">Import from a URL</h2>
    <p class="mt-1 text-neutral-500">
      Any page publishing schema.org/Recipe data. Fetched server-side, parsed in
      your browser.
    </p>

    <form onsubmit={importUrlSubmit} class="mt-4 flex gap-2">
      <input
        bind:value={importUrl}
        placeholder="https://example.com/recipes/…"
        aria-label="Recipe URL"
        class="flex-1 rounded-lg border border-neutral-300 px-3 py-2 outline-hidden focus:border-neutral-900"
      />
      <button
        disabled={importing}
        class="rounded-lg bg-neutral-900 px-4 py-2 font-medium text-white hover:bg-neutral-700 disabled:opacity-50"
      >
        Import
      </button>
    </form>

    <ImportResultView result={imported} pending={importing} />
  </section>
</main>
