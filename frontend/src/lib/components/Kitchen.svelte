<script lang="ts">
  import type { KitchenDetail, KitchensStatus } from "$lib/types";

  /**
   * One kitchen (#72): what it is and who is in it. Everything you *do* to it — invite
   * someone, rename it, stock it — is its own page, so this one stays a single idea.
   *
   * A primary kitchen says so. It is the one made for you and the one the app assumes,
   * so the difference between it and a kitchen you opened on purpose is worth naming.
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
  <div class="rounded-card bg-cream-50 p-6">
    <a href="/kitchens" class="text-sm text-stone-500 underline">← Kitchens</a>

    {#if status === "error" || (status === "ready" && !kitchen)}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't open this kitchen."}
      </p>
    {:else if status === "pending" || !kitchen}
      <div class="rounded-card mt-4 h-10 w-full bg-stone-100" aria-hidden="true"></div>
    {:else}
      <h1 class="font-display mt-3 text-2xl font-medium text-stone-900">
        {kitchen.name}
      </h1>
      {#if kitchen.is_primary}
        <p class="mt-1 text-xs text-stone-500">
          Yours by default — the kitchen the app works in until you open another.
        </p>
      {/if}

      <button
        type="button"
        onclick={onPlan}
        class="rounded-card font-display bg-cocoa-500 text-cream-50 mt-5 flex w-full items-center justify-between px-4 py-3"
      >
        Plan a meal here
        <span class="text-cream-200 text-sm">→</span>
      </button>

      <ul class="mt-5 flex flex-col gap-2">
        <li>
          <a
            href="/kitchens/{kitchen.id}/name"
            class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
          >
            Rename
            <span class="text-sm text-stone-400">→</span>
          </a>
        </li>
        <li>
          <a
            href="/kitchens/{kitchen.id}/invite"
            class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
          >
            Invite someone
            <span class="text-sm text-stone-400">→</span>
          </a>
        </li>
        <li>
          <a
            href="/kitchens/{kitchen.id}/equipment"
            class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
          >
            Equipment
            <span class="text-sm text-stone-400">{kitchen.equipment.length} →</span>
          </a>
        </li>
        <li>
          <a
            href="/kitchens/{kitchen.id}/pantry"
            class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
          >
            Pantry
            <span class="text-sm text-stone-400">{kitchen.pantry.length} →</span>
          </a>
        </li>
      </ul>

      <p class="mt-8 mb-3 text-xs text-stone-500">Who's in it</p>
      <ul class="flex flex-col gap-1.5">
        {#each kitchen.members as m (m.telegram_user_id)}
          <li class="font-display text-stone-900">
            {m.username ? `@${m.username}` : m.telegram_user_id}
          </li>
        {/each}
      </ul>

    {/if}
  </div>
</div>
