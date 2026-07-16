<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { completeLogin } from "$lib/auth";

  /**
   * Where the bot's link lands. It carries the one secret that redeems a
   * session, so opening it here is what signs *this browser* in — which is the
   * point of the design: the session goes to whoever opened the bot's link, and
   * the bot only ever sends it to the person who messaged it.
   */
  let error = $state<string | null>(null);

  onMount(async () => {
    const c = page.url.searchParams.get("c");
    if (!c) {
      error = "That link is missing its code.";
      return;
    }
    try {
      await completeLogin(c);
      // Replace, so Back does not land on a spent secret — and so the secret
      // stops sitting in the address bar.
      await goto("/", { replaceState: true });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  });
</script>

<div class="mx-auto flex max-w-md flex-col items-center px-4 py-16 text-center">
  <h1 class="font-display text-4xl font-medium tracking-tight text-stone-900">
    recipes
  </h1>
  {#if error}
    <p class="text-paprika-500 mt-6">{error}</p>
    <p class="mt-2 text-sm text-stone-500">
      Send <span class="font-medium">/start</span> to the bot again for a fresh link.
    </p>
    <a
      href="/"
      class="mt-4 text-sm text-stone-500 underline hover:text-stone-900"
    >
      Back to the site
    </a>
  {:else}
    <p class="mt-6 text-stone-500">Signing you in…</p>
  {/if}
</div>
