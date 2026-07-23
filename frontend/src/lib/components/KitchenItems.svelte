<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import type { KitchensStatus } from "$lib/types";

  /**
   * A kitchen's equipment, or its pantry (#72) — the same page twice, so it is one
   * component. Nothing here but the list and adding to it.
   *
   * When `options` is given the field becomes a **picker**: you narrow the known list
   * and choose from it, and there is no way to add something that is not on it. That
   * is the equipment rule (#81) — a kitchen selects from what recipes actually ask
   * for, because owning something no recipe mentions could never change what you are
   * able to cook. The pantry has no vocabulary yet and stays free text.
   */
  interface Props {
    status: KitchensStatus;
    /** "Equipment" or "Pantry". */
    title: string;
    items?: string[];
    /** Where the add field points the user, e.g. "blender, wok…". */
    placeholder: string;
    /**
     * The only things that may be added, when there is such a list. Absent means free
     * text; present-but-empty means nothing can be added yet, which is a real state —
     * the corpus has not been read, so there is genuinely nothing legitimate to pick.
     */
    options?: string[];
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
    options,
    backHref,
    error,
    actionError,
    onAdd,
    onRemove,
  }: Props = $props();

  let value = $state("");

  /** What is left to offer: known items this kitchen does not already have. */
  const available = $derived(
    options?.filter((o) => !items.includes(o)) ?? [],
  );

  /** Narrowed by what has been typed — a picker, not a search. */
  const matches = $derived(
    value.trim()
      ? available.filter((o) => o.includes(value.trim().toLowerCase()))
      : available,
  );

  /** Only a name on the list counts, so the field cannot invent one. */
  const choosable = $derived(
    options === undefined || options.includes(value.trim().toLowerCase()),
  );

  async function add(e: Event) {
    e.preventDefault();
    const v = value.trim();
    if (!v || !choosable) return;
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

      {#if options !== undefined && options.length === 0}
        <p class="mt-6 text-sm text-stone-600">
          Nothing to choose from yet — no recipe has been read for the equipment it
          needs. Once the corpus has been read, what you can own appears here.
        </p>
      {:else}
        <form class="mt-6 flex gap-2" onsubmit={add}>
          <input
            bind:value
            {placeholder}
            list={options ? "known-items" : undefined}
            class="rounded-card flex-1 border border-stone-200 bg-cream-50 px-3 py-2 text-sm text-stone-900"
          />
          {#if options}
            <datalist id="known-items">
              {#each matches as option (option)}
                <option value={option}></option>
              {/each}
            </datalist>
          {/if}
          <button
            type="submit"
            disabled={!choosable}
            class="rounded-card flex-none bg-cocoa-500 px-4 py-2 text-sm font-medium text-cream-50 disabled:opacity-40"
          >
            Add
          </button>
        </form>
        {#if options && value.trim() && !choosable}
          <p class="mt-2 text-sm text-stone-500">
            Not something any recipe asks for. Pick from the list.
          </p>
        {/if}
      {/if}
    {/if}
  </Panel>
</div>
