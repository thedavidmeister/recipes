<script lang="ts">
  import type {
    KitchenDetail,
    KitchenSummary,
    KitchensStatus,
  } from "$lib/types";

  /**
   * `kitchens` (#72): the durable shared space that scopes the meal flow — who's in
   * it (owner + guests), and what it's stocked with (equipment + pantry).
   *
   * Presentational: the page owns the queries and the mutations (create, join, add/
   * remove) and passes the list, the open kitchen's detail, and callbacks. The only
   * state here is the ephemeral text of the input fields.
   */
  interface Props {
    status: KitchensStatus;
    /** The kitchens the user belongs to. */
    kitchens?: KitchenSummary[];
    /** The open kitchen in full, or `null` when none is selected. */
    selected?: KitchenDetail | null;
    /** The shareable invite URL for the open kitchen (the page builds it). */
    inviteLink?: string;
    error?: string;
    onCreate?: (name: string) => void;
    onJoin?: (token: string) => void;
    onSelect?: (id: string) => void;
    onAddEquipment?: (item: string) => void;
    onRemoveEquipment?: (item: string) => void;
    onAddPantry?: (item: string) => void;
    onRemovePantry?: (item: string) => void;
  }

  let {
    status,
    kitchens = [],
    selected,
    inviteLink,
    error,
    onCreate,
    onJoin,
    onSelect,
    onAddEquipment,
    onRemoveEquipment,
    onAddPantry,
    onRemovePantry,
  }: Props = $props();

  let newName = $state("");
  let joinToken = $state("");
  let newEquipment = $state("");
  let newPantry = $state("");
  let copied = $state(false);

  function submit(value: string, run?: (v: string) => void, clear?: () => void) {
    const v = value.trim();
    if (v) {
      run?.(v);
      clear?.();
    }
  }

  async function copyInvite() {
    if (!inviteLink) return;
    try {
      await navigator.clipboard.writeText(inviteLink);
      copied = true;
    } catch {
      // Clipboard blocked — the link is visible to copy by hand.
    }
  }
</script>

<div class="pt-6">
  <header class="mb-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-cocoa-500" aria-hidden="true"></span>
      Kitchens
    </p>
    <p class="mt-1 text-sm text-stone-500">
      The space your cooking happens in — who's in it, and what it's stocked with.
    </p>
  </header>

  {#if status === "error"}
    <div class="rounded-card border border-paprika-500/30 bg-paprika-100 p-6">
      <p class="font-display text-stone-900">Couldn't load your kitchens.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the backend."}
      </p>
    </div>
  {:else if status === "pending"}
    <div class="rounded-card h-10 w-full bg-stone-100" aria-hidden="true"></div>
  {:else}
    <!-- The kitchens you're in, plus creating and joining one. -->
    {#if kitchens.length}
      <ul class="mb-4 flex flex-wrap gap-2">
        {#each kitchens as k (k.id)}
          <li>
            <button
              type="button"
              onclick={() => onSelect?.(k.id)}
              class="rounded-pill px-3 py-1 text-sm {selected?.id === k.id
                ? 'bg-cocoa-500 text-cream-50'
                : 'border border-stone-200 bg-cream-100 text-stone-700'}"
            >
              {k.name}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="mb-8 flex flex-col gap-2 sm:flex-row">
      <form
        class="flex flex-1 gap-2"
        onsubmit={(e) => {
          e.preventDefault();
          submit(newName, onCreate, () => (newName = ""));
        }}
      >
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
      <form
        class="flex flex-1 gap-2"
        onsubmit={(e) => {
          e.preventDefault();
          submit(joinToken, onJoin, () => (joinToken = ""));
        }}
      >
        <input
          bind:value={joinToken}
          placeholder="Paste an invite"
          class="rounded-card flex-1 border border-stone-200 bg-cream-50 px-3 py-2 text-sm text-stone-900"
        />
        <button
          type="submit"
          class="rounded-card flex-none border border-cocoa-500 px-4 py-2 text-sm font-medium text-cocoa-500"
        >
          Join
        </button>
      </form>
    </div>

    {#if selected}
      <section>
        <div class="flex items-center gap-3">
          <h1 class="font-display text-2xl font-medium text-stone-900">
            {selected.name}
          </h1>
          <span
            class="rounded-pill flex-none border border-cocoa-500 px-2.5 py-0.5 text-xs text-cocoa-500"
          >
            {selected.role}
          </span>
        </div>

        {#if inviteLink}
          <div
            class="rounded-card mt-4 flex items-center gap-3 border border-stone-200 bg-cream-100 px-4 py-3"
          >
            <span class="flex-1 truncate text-sm text-stone-600">{inviteLink}</span>
            <button
              type="button"
              onclick={copyInvite}
              class="rounded-pill flex-none border border-cocoa-500 px-3 py-1 text-sm text-cocoa-500"
            >
              {copied ? "Copied" : "Copy invite"}
            </button>
          </div>
        {/if}

        <!-- Who's here -->
        <h2 class="font-display mt-8 mb-3 flex items-center gap-2 text-stone-600">
          <span class="size-2 rounded-full bg-cocoa-500" aria-hidden="true"></span>
          Who's here
        </h2>
        <ul class="flex flex-col gap-2">
          {#each selected.members as m (m.telegram_user_id)}
            <li class="flex items-center gap-2 text-stone-900">
              <span class="font-display">
                {m.username ? `@${m.username}` : m.telegram_user_id}
              </span>
              <span class="text-sm text-stone-400">— {m.role}</span>
            </li>
          {/each}
        </ul>

        <!-- Equipment -->
        <h2 class="font-display mt-8 mb-3 flex items-center gap-2 text-stone-600">
          <span class="size-2 rounded-full bg-cocoa-500" aria-hidden="true"></span>
          Equipment
        </h2>
        {#if selected.equipment.length}
          <ul class="mb-3 flex flex-wrap gap-2">
            {#each selected.equipment as item (item)}
              <li
                class="rounded-pill flex items-center gap-2 border border-stone-200 bg-cream-100 px-3 py-1 text-sm text-stone-700"
              >
                {item}
                <button
                  type="button"
                  aria-label={`Remove ${item}`}
                  onclick={() => onRemoveEquipment?.(item)}
                  class="text-stone-400">×</button
                >
              </li>
            {/each}
          </ul>
        {:else}
          <p class="mb-3 text-sm text-stone-500">Nothing tracked yet.</p>
        {/if}
        <form
          class="flex gap-2"
          onsubmit={(e) => {
            e.preventDefault();
            submit(newEquipment, onAddEquipment, () => (newEquipment = ""));
          }}
        >
          <input
            bind:value={newEquipment}
            placeholder="Add equipment (blender, wok…)"
            class="rounded-card flex-1 border border-stone-200 bg-cream-50 px-3 py-2 text-sm text-stone-900"
          />
          <button
            type="submit"
            class="rounded-card flex-none border border-cocoa-500 px-4 py-2 text-sm font-medium text-cocoa-500"
          >
            Add
          </button>
        </form>

        <!-- Pantry -->
        <h2 class="font-display mt-8 mb-3 flex items-center gap-2 text-stone-600">
          <span class="size-2 rounded-full bg-cocoa-500" aria-hidden="true"></span>
          Pantry
        </h2>
        {#if selected.pantry.length}
          <ul class="mb-3 flex flex-wrap gap-2">
            {#each selected.pantry as item (item)}
              <li
                class="rounded-pill flex items-center gap-2 border border-stone-200 bg-cream-100 px-3 py-1 text-sm text-stone-700"
              >
                {item}
                <button
                  type="button"
                  aria-label={`Remove ${item}`}
                  onclick={() => onRemovePantry?.(item)}
                  class="text-stone-400">×</button
                >
              </li>
            {/each}
          </ul>
        {:else}
          <p class="mb-3 text-sm text-stone-500">Nothing on hand yet.</p>
        {/if}
        <form
          class="flex gap-2"
          onsubmit={(e) => {
            e.preventDefault();
            submit(newPantry, onAddPantry, () => (newPantry = ""));
          }}
        >
          <input
            bind:value={newPantry}
            placeholder="Add to the pantry (rice, eggs…)"
            class="rounded-card flex-1 border border-stone-200 bg-cream-50 px-3 py-2 text-sm text-stone-900"
          />
          <button
            type="submit"
            class="rounded-card flex-none border border-cocoa-500 px-4 py-2 text-sm font-medium text-cocoa-500"
          >
            Add
          </button>
        </form>
      </section>
    {:else if kitchens.length === 0}
      <div class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center">
        <p class="font-display text-stone-900">No kitchens yet.</p>
        <p class="mt-1 text-sm text-stone-600">
          Create one above — a home, a share house, a holiday rental — then invite the
          people you cook with.
        </p>
      </div>
    {/if}
  {/if}
</div>
