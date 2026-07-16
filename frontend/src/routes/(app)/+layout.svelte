<script lang="ts">
  import { page } from "$app/state";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { me, logout, botLink } from "$lib/auth";
  import type { LoginStatus, Section } from "$lib/types";
  import Login from "$lib/components/Login.svelte";
  import Nav from "$lib/components/Nav.svelte";

  let { children } = $props();
  const queryClient = useQueryClient();

  /**
   * The auth gate for everything in this group.
   *
   * It lives here rather than per-page because auth is mandatory (#25) — a gate
   * you have to remember to add to each new page is one you will eventually
   * forget. `/auth/finish` is deliberately **outside** this group: it is how a
   * session is obtained, so gating it would deadlock the login.
   *
   * The session is an HttpOnly cookie, so script cannot answer this locally;
   * only the server knows. `retry: false` because a 401 is a legitimate answer
   * ("nobody is logged in"), not a failure worth retrying.
   *
   * Polling while signed out is also how a tab notices a login: opening the
   * bot's link in the same browser sets the cookie, and the next poll simply
   * starts succeeding.
   */
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
    retry: false,
    refetchInterval: (q) => (q.state.data ? false : 2000),
  }));

  const authed = $derived(!!session.data);
  const loginStatus = $derived<LoginStatus>(
    session.isError ? "error" : session.isPending ? "checking" : "idle",
  );

  // The first path segment is the section. Anything else has no business here.
  const current = $derived(
    (page.url.pathname.split("/")[1] || "pick") as Section,
  );

  async function signOut() {
    await logout();
    queryClient.clear();
  }
</script>

{#if !authed}
  <Login
    status={loginStatus}
    link={botLink()}
    error={session.error instanceof Error ? session.error.message : undefined}
  />
{:else}
  <!--
    The nav is the heading: `pick · buy · cook · joy` names where you are more
    clearly than an <h1> repeating the same word underneath it would. So the
    line goes first and the page starts below it.
  -->
  <Nav {current} />

  <div class="mx-auto max-w-2xl px-4 pb-16">
    <div class="flex justify-end gap-3 py-2 text-sm">
      {#if session.data?.username}
        <span class="text-stone-500">@{session.data.username}</span>
      {/if}
      <button
        onclick={signOut}
        class="text-stone-500 underline hover:text-stone-900"
      >
        Sign out
      </button>
    </div>

    {@render children()}
  </div>
{/if}
