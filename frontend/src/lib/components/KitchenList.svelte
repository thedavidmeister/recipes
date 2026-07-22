<script lang="ts">
  import type { KitchenSummary, KitchensStatus } from "$lib/types";

  /**
   * `kitchens` (#72): the kitchens you're in, and making a new one. That is the whole
   * page — opening one is a navigation, not a state change here, so this never has to
   * know what a kitchen contains.
   */
  interface Props {
    status: KitchensStatus;
    kitchens?: KitchenSummary[];
    error?: string;
    actionError?: string;
    onCreate?: (name: string) => void | Promise<void>;
  }

  let { status, kitchens = [], error, actionError, onCreate }: Props = $props();

  const owned = $derived(kitchens.filter((k) => k.role === "owner"));
  const guest = $derived(kitchens.filter((k) => k.role !== "owner"));

  let newName = $state("");

  // Clear the field only once the create has landed — emptying it up front reads as
  // success even when it failed.
  async function create(e: Event) {
    e.preventDefault();
    const v = newName.trim();
    if (!v) return;
    try {
      await onCreate?.(v);
    } catch {
      return;
    }
    newName = "";
  }
</script>

<div class="pt-48 pb-16">
  <div class="rounded-card bg-cream-50 p-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-cocoa-500" aria-hidden="true"></span>
      Kitchens
    </p>

    {#if status === "error"}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't load your kitchens."}
      </p>
    {:else if status === "pending"}
      <div class="rounded-card mt-4 h-10 w-full bg-stone-100" aria-hidden="true"></div>
    {:else}
      {#if actionError}
        <p role="alert" class="mt-3 text-sm text-paprika-500">{actionError}</p>
      {/if}

      {#snippet picker(label: string, items: KitchenSummary[])}
        {#if items.length}
          <p class="mt-5 mb-2 text-xs text-stone-500">{label}</p>
          <ul class="flex flex-col gap-2">
            {#each items as k (k.id)}
              <li>
                <a
                  href="/kitchens/{k.id}"
                  class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
                >
                  {k.name}
                  <span class="text-sm text-stone-400">→</span>
                </a>
              </li>
            {/each}
          </ul>
        {/if}
      {/snippet}
      {@render picker("Your kitchens", owned)}
      {@render picker("Friends' kitchens", guest)}

      {#if kitchens.length === 0}
        <p class="mt-4 text-sm text-stone-600">
          No kitchens yet. Make one — a home, a share house, a holiday rental — then
          invite the people you cook with.
        </p>
      {/if}

      <form class="mt-6 flex gap-2" onsubmit={create}>
        <input
          bind:value={newName}
          placeholder="New kitchen name"
          class="rounded-card flex-1 border border-stone-200 bg-cream-50 px-3 py-2 text-sm text-stone-900"
        />
        <button
          type="submit"
          class="rounded-card flex-none bg-cocoa-500 px-4 py-2 text-sm font-medium text-cream-50"
        >
          Create
        </button>
      </form>
    {/if}
  </div>
</div>
