<script lang="ts">
  import { onMount } from "svelte";
  import { useQueryClient } from "@tanstack/svelte-query";
  import { goto } from "$app/navigation";
  import { ApiError } from "$lib/client";
  import { createPick } from "$lib/pick";

  /**
   * `pick` (#20): a live, shared swipe-and-vote over the corpus.
   *
   * There is no separate "start" screen and no solo/group split — pick *is* the
   * swipe. Landing here mints a fresh pick and hands off to its room
   * (`/pick/[channel]`), whose URL is the shareable invite; opening someone else's
   * link joins their pick instead. The deck is fed by the walk engine (#47) behind
   * the scenes, in each client's own order.
   */
  const queryClient = useQueryClient();
  let error = $state<string | null>(null);

  onMount(() => {
    void start();
  });

  async function start() {
    error = null;
    try {
      const channel = await createPick();
      await goto(`/pick/${channel}`, { replaceState: true });
    } catch (e) {
      // A lapsed session 401s the start — drop back to login, the only recovery.
      if (e instanceof ApiError && e.status === 401) {
        queryClient.invalidateQueries({ queryKey: ["session"] });
        return;
      }
      error = e instanceof Error ? e.message : "Could not start a pick.";
    }
  }
</script>

<div class="pt-6">
  {#if error}
    <div class="rounded-card border border-paprika-500/30 bg-paprika-100 p-6">
      <p class="font-display text-stone-900">Could not start a pick.</p>
      <p class="mt-1 text-sm text-stone-600">{error}</p>
      <button
        onclick={start}
        class="mt-4 text-sm font-medium text-paprika-500 underline hover:text-stone-900"
      >
        Try again
      </button>
    </div>
  {:else}
    <div
      class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
    >
      <p
        class="font-display flex items-center justify-center gap-2 text-stone-900"
      >
        <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"
        ></span>
        Starting a pick…
      </p>
    </div>
  {/if}
</div>
