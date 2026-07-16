<script lang="ts">
  import { Select } from "bits-ui";
  import type { Category } from "$lib/types";

  let {
    categories,
    value = $bindable(""),
    onSelect,
  }: {
    categories: Category[];
    value?: string;
    onSelect?: (category: string) => void;
  } = $props();

  const label = $derived(value || "Browse a category…");

  function select(next: string) {
    value = next;
    onSelect?.(next);
  }
</script>

<Select.Root type="single" bind:value onValueChange={select}>
  <Select.Trigger
    aria-label="Browse a category"
    class="flex w-full items-center justify-between rounded-xl border border-stone-300 px-4 py-2.5 text-left hover:border-stone-400"
  >
    <span class={value ? "" : "text-stone-500"}>{label}</span>
    <span aria-hidden="true" class="ml-2 text-stone-400">▾</span>
  </Select.Trigger>
  <Select.Portal>
    <Select.Content
      class="bg-cream-100 z-50 max-h-72 overflow-y-auto rounded-2xl border border-stone-200 py-1 shadow-lg"
      sideOffset={4}
    >
      <Select.Viewport>
        {#each categories as category (category.name)}
          <Select.Item
            value={category.name}
            label={category.name}
            class="cursor-pointer px-3 py-2 data-highlighted:bg-stone-100"
          >
            {#snippet children({ selected })}
              <span class={selected ? "font-semibold" : ""}
                >{category.name}</span
              >
            {/snippet}
          </Select.Item>
        {/each}
      </Select.Viewport>
    </Select.Content>
  </Select.Portal>
</Select.Root>
