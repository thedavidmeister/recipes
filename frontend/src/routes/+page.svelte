<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import {
    searchThemealdb,
    listCategories,
    browseCategory,
  } from "$lib/sources";
  import { me, startLogin, pollLogin, logout } from "$lib/auth";
  import type { LoginStatus, SearchStatus, LoginStart } from "$lib/types";
  import SearchResults from "$lib/components/SearchResults.svelte";
  import CategoryPicker from "$lib/components/CategoryPicker.svelte";
  import Login from "$lib/components/Login.svelte";

  const queryClient = useQueryClient();

  // Auth is mandatory (#25), so this gates the whole page — search included,
  // because since #29 a search *is* an ingest.
  //
  // The session is an HttpOnly cookie, so script cannot answer this locally;
  // only the server knows. `retry: false` because a 401 is a legitimate answer
  // ("nobody is logged in"), not a failure worth retrying.
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
    retry: false,
  }));

  // One state machine rather than several flags, so the states the story file
  // declares are the states that exist.
  let phase = $state<Exclude<LoginStatus, "checking">>("idle");
  let attempt = $state<LoginStart | null>(null);
  let loginError = $state<string | null>(null);

  // `checking` is the query's business; every other phase is ours.
  const loginStatus = $derived<LoginStatus>(
    session.isPending ? "checking" : phase,
  );

  async function beginLogin() {
    loginError = null;
    phase = "starting";
    try {
      attempt = await startLogin();
      phase = "waiting";
    } catch (e) {
      loginError = e instanceof Error ? e.message : String(e);
      phase = "error";
    }
  }

  // Wait for the tap.
  //
  // Polling, because the alternative — waiting on a socket — is #20's to build;
  // when it lands this can wait on that instead (see #25). The nonce is
  // short-lived, so this cannot spin forever: it stops on ready, on expiry, or
  // when the component tears down.
  $effect(() => {
    if (phase !== "waiting" || !attempt) return;
    const secret = attempt.pollSecret;

    let live = true;
    const timer = setInterval(async () => {
      if (!live) return;
      try {
        const result = await pollLogin(secret);
        if (!live) return;
        if (result.status === "ready") {
          live = false;
          attempt = null;
          phase = "idle";
          // The cookie arrived on that response and script cannot read it, so
          // there is nothing to store — just ask the server who we are now.
          await queryClient.invalidateQueries({ queryKey: ["session"] });
        } else if (result.status === "expired") {
          live = false;
          attempt = null;
          phase = "expired";
        }
      } catch (e) {
        live = false;
        loginError = e instanceof Error ? e.message : String(e);
        phase = "error";
      }
    }, 2000);

    return () => {
      live = false;
      clearInterval(timer);
    };
  });

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
    link={attempt?.link}
    error={loginError ?? undefined}
    onStart={beginLogin}
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
