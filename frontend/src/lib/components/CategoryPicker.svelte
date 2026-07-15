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
    class="flex w-full items-center justify-between rounded-lg border border-neutral-300 px-3 py-2 text-left hover:border-neutral-400"
  >
    <span class={value ? "" : "text-neutral-500"}>{label}</span>
    <span aria-hidden="true" class="ml-2 text-neutral-400">▾</span>
  </Select.Trigger>
  <Select.Portal>
    <Select.Content
      class="z-50 max-h-72 overflow-y-auto rounded-lg border border-neutral-200 bg-white py-1 shadow-lg"
      sideOffset={4}
    >
      <Select.Viewport>
        {#each categories as category (category.name)}
          <Select.Item
            value={category.name}
            label={category.name}
            class="cursor-pointer px-3 py-2 data-highlighted:bg-neutral-100"
          >
            {#snippet children({ selected })}
              <span class={selected ? "font-semibold" : ""}>{category.name}</span>
            {/snippet}
          </Select.Item>
        {/each}
      </Select.Viewport>
    </Select.Content>
  </Select.Portal>
</Select.Root>
