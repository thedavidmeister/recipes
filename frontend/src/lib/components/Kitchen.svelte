<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";

  /**
   * One kitchen (#72): what it is, who is in it, and the thing you come here to do —
   * start a meal. Changing the kitchen — its name, who's in it, what it owns — lives
   * on its settings page (#117), reached by a single quiet link, so this page is about
   * being in the kitchen rather than administering it.
   *
   * Nobody here has a role. Everyone in a kitchen is an owner of it — being a guest is
   * something you are at a meal, not in a room — so the list of people is a list of
   * people.
   */
  interface Props {
    status: KitchensStatus;
    kitchen?: KitchenDetail | null;
    error?: string;
    /** Start a meal plan in this kitchen — the lobby is the next page. */
    onPlan?: () => void;
  }

  let { status, kitchen, error, onPlan }: Props = $props();
</script>

<div class="pt-48 pb-16">
  <Panel>
    <a href="/kitchens" class="text-sm text-stone-500 underline">← Kitchens</a>

    {#if status === "error" || (status === "ready" && !kitchen)}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't open this kitchen."}
      </p>
    {:else if status === "pending" || !kitchen}
      <div class="mt-4"><Skeleton /></div>
    {:else}
      <h1 class="font-display mt-3 text-2xl font-medium text-stone-900">
        {kitchen.name}
      </h1>

      <div class="mt-5">
        <Button onclick={onPlan} dot="pesto">Let's cook!</Button>
      </div>

      <p class="mt-8 mb-3 text-xs text-stone-500">Who's in it</p>
      <ul class="flex flex-col gap-1.5">
        {#each kitchen.members as m (m.telegram_user_id)}
          <li class="font-display text-stone-900">
            {m.username ? `@${m.username}` : m.telegram_user_id}
          </li>
        {/each}
      </ul>

      <div class="mt-8">
        <a
          href="/kitchens/{kitchen.id}/settings"
          class="text-sm text-stone-500 underline hover:text-stone-700"
        >
          Settings
        </a>
      </div>
    {/if}
  </Panel>
</div>
