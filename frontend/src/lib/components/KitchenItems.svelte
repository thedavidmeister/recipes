<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import type { KitchensStatus } from "$lib/types";

  /**
   * A kitchen's equipment, or its pantry (#72) — the same page twice, so it is one
   * component. Nothing here but the list and adding to it.
   */
  interface Props {
    status: KitchensStatus;
    /** "Equipment" or "Pantry". */
    title: string;
    items?: string[];
    /** Where the add field points the user, e.g. "blender, wok…". */
    placeholder: string;
    /** Back to the kitchen this belongs to. */
    backHref: string;
    error?: string;
    actionError?: string;
    onAdd?: (item: string) => void | Promise<void>;
    onRemove?: (item: string) => void | Promise<void>;
  }

  let {
    status,
    title,
    items = [],
    placeholder,
    backHref,
    error,
    actionError,
    onAdd,
    onRemove,
  }: Props = $props();

  let value = $state("");

  async function add(e: Event) {
    e.preventDefault();
    const v = value.trim();
    if (!v) return;
    try {
      await onAdd?.(v);
    } catch {
      return;
    }
    value = "";
  }

  // The page has already put the reason on `actionError`; an uncaught throw here
  // would just be an unhandled rejection.
  function remove(item: string) {
    void Promise.resolve(onRemove?.(item)).catch(() => {});
  }
</script>

<div class="pt-48 pb-16">
  <Panel>
    <a href={backHref} class="text-sm text-stone-500 underline">← Kitchen</a>
    <h1 class="font-display mt-3 text-2xl font-medium text-stone-900">{title}</h1>

    {#if status === "error"}
      <p class="mt-4 text-sm text-stone-600">{error ?? `Couldn't load the ${title.toLowerCase()}.`}</p>
    {:else if status === "pending"}
      <div class="mt-4"><Skeleton /></div>
    {:else}
      {#if actionError}
        <p role="alert" class="mt-3 text-sm text-paprika-500">{actionError}</p>
      {/if}

      {#if items.length}
        <ul class="mt-5 flex flex-wrap gap-2">
          {#each items as item (item)}
            <li
              class="rounded-pill flex items-center gap-2 border border-stone-200 bg-cream-100 px-3 py-1 text-sm text-stone-700"
            >
              {item}
              <button
                type="button"
                aria-label={`Remove ${item}`}
                onclick={() => remove(item)}
                class="text-stone-400">×</button
              >
            </li>
          {/each}
        </ul>
      {:else}
        <p class="mt-5 text-sm text-stone-600">Nothing here yet.</p>
      {/if}

      <form class="mt-6 flex gap-2" onsubmit={add}>
        <input
          bind:value
          {placeholder}
          class="rounded-card flex-1 border border-stone-200 bg-cream-50 px-3 py-2 text-sm text-stone-900"
        />
        <button
          type="submit"
          class="rounded-card flex-none bg-cocoa-500 px-4 py-2 text-sm font-medium text-cream-50"
        >
          Add
        </button>
      </form>
    {/if}
  </Panel>
</div>
