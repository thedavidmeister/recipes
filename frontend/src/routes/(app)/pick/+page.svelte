<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import {
    searchThemealdb,
    listCategories,
    browseCategory,
  } from "$lib/sources";
  import type { SearchStatus } from "$lib/types";
  import SearchResults from "$lib/components/SearchResults.svelte";
  import CategoryPicker from "$lib/components/CategoryPicker.svelte";

  /**
   * Choosing what to eat. #20 — the group decide — is what this is for; the
   * search and browse below are the scaffolding that existed before that, kept
   * because they work, not because they are the plan.
   */
  let term = $state("");
  let submitted = $state("");
  let category = $state("");

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

  // Searching and browsing are one result list: the last action wins.
  const active = $derived(category ? browsed : results);
  const label = $derived(category || submitted);

  const searchStatus = $derived<SearchStatus>(
    !label
      ? "idle"
      : active.isError
        ? "error"
        : active.isPending
          ? "pending"
          : "ready",
  );

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
</script>

<form onsubmit={search} class="mt-6 flex gap-2">
  <input
    bind:value={term}
    placeholder="chicken, pasta, curry…"
    aria-label="Search recipes"
    class="flex-1 rounded-xl border border-stone-300 px-4 py-2.5 outline-hidden focus:border-stone-900"
  />
  <button
    class="bg-cream-50 font-display flex items-center gap-2 rounded-xl border-2 border-stone-300 px-5 py-2.5 font-semibold text-stone-900 transition hover:border-stone-400"
  >
    <span class="bg-pesto-500 size-2.5 rounded-full"></span>
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

<SearchResults status={searchStatus} recipes={active.data ?? []} term={label} />
