<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { searchThemealdb } from "$lib/sources";
  import type { SearchStatus } from "$lib/types";
  import SearchResults from "$lib/components/SearchResults.svelte";

  let term = $state("");
  let submitted = $state("");

  const results = createQuery(() => ({
    queryKey: ["themealdb", submitted],
    queryFn: () => searchThemealdb(submitted),
    enabled: submitted.length > 0,
  }));

  const status = $derived<SearchStatus>(
    !submitted
      ? "idle"
      : results.isError
        ? "error"
        : results.isPending
          ? "pending"
          : "ready",
  );

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
      class="flex-1 rounded-lg border border-neutral-300 px-3 py-2 outline-hidden focus:border-neutral-900"
    />
    <button
      class="rounded-lg bg-neutral-900 px-4 py-2 font-medium text-white hover:bg-neutral-700"
    >
      Search
    </button>
  </form>

  <SearchResults {status} recipes={results.data ?? []} term={submitted} />
</main>
