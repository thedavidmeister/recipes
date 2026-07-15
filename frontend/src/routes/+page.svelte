<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import {
    searchThemealdb,
    listCategories,
    browseCategory,
  } from "$lib/sources";
  import { me, logout, botLink } from "$lib/auth";
  import type { LoginStatus, SearchStatus } from "$lib/types";
  import SearchResults from "$lib/components/SearchResults.svelte";
  import CategoryPicker from "$lib/components/CategoryPicker.svelte";
  import Login from "$lib/components/Login.svelte";

  const queryClient = useQueryClient();

  // Auth is mandatory (#25), so this gates the whole page — search included,
  // because since #29 a search *is* an ingest.
  //
  // The session is an HttpOnly cookie, so script cannot answer this locally; only
  // the server knows. `retry: false` because a 401 is a legitimate answer
  // ("nobody is logged in"), not a failure worth retrying.
  //
  // It refetches while signed out, which is also how this tab notices a login:
  // opening the bot's link in the same browser sets the cookie, and the next poll
  // simply starts succeeding. There is nothing to coordinate — the tab that
  // showed the link holds no secret, deliberately.
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
    retry: false,
    refetchInterval: (q) => (q.state.data ? false : 2000),
  }));

  const loginStatus = $derived<LoginStatus>(
    session.isError ? "error" : session.isPending ? "checking" : "idle",
  );

  async function signOut() {
    await logout();
    // Drop the corpus queries too: they were fetched as someone.
    queryClient.removeQueries({ queryKey: ["themealdb"] });
    await queryClient.invalidateQueries({ queryKey: ["session"] });
    submitted = "";
    category = "";
    term = "";
  }

  let term = $state("");
  let submitted = $state("");
  let category = $state("");

  const authed = $derived(!!session.data);

  // Searching and browsing are one result list: the last action wins. Both wait
  // on a session — without one every call 401s, so firing them would only
  // manufacture errors.
  const results = createQuery(() => ({
    queryKey: ["themealdb", "search", submitted],
    queryFn: () => searchThemealdb(submitted),
    enabled: authed && submitted.length > 0,
  }));

  const browsed = createQuery(() => ({
    queryKey: ["themealdb", "category", category],
    queryFn: () => browseCategory(category),
    enabled: authed && category.length > 0,
  }));

  const categories = createQuery(() => ({
    queryKey: ["themealdb", "categories"],
    queryFn: listCategories,
    enabled: authed,
  }));

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

{#if !authed}
  <Login
    status={loginStatus}
    link={botLink()}
    error={session.error instanceof Error ? session.error.message : undefined}
  />
{:else}
  <main class="mx-auto max-w-5xl px-4 py-10">
    <div class="flex items-baseline justify-between gap-4">
      <div>
        <h1 class="text-3xl font-bold tracking-tight">recipes</h1>
        <p class="mt-1 text-neutral-500">Search a public recipe database.</p>
      </div>
      <div class="flex shrink-0 items-baseline gap-3 text-sm">
        {#if session.data?.username}
          <span class="text-neutral-500">@{session.data.username}</span>
        {/if}
        <button
          onclick={signOut}
          class="text-neutral-500 underline hover:text-neutral-900"
        >
          Sign out
        </button>
      </div>
    </div>

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

    <SearchResults
      status={searchStatus}
      recipes={active.data ?? []}
      term={label}
    />
  </main>
{/if}
