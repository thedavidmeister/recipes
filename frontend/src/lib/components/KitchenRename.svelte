<script lang="ts">
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  import { untrack } from "svelte";

  /**
   * Renaming a kitchen (#72) — a page, like making one, because it is a thing you are
   * doing rather than a corner of the kitchen you are looking at.
   *
   * This is the whole of what you do to a primary kitchen that you would otherwise
   * have done by creating one: it arrives named after you, and here is where it stops
   * being.
   */
  interface Props {
    current: string;
    error?: string;
    onRename: (name: string) => void | Promise<void>;
  }

  let { current, error, onRename }: Props = $props();

  // Seeded from the name it has, then it is yours: a refetch landing a new name while
  // you are mid-word must not overwrite what you are typing.
  let name = $state(untrack(() => current));
  let saving = $state(false);

  async function submit(e: Event) {
    e.preventDefault();
    const v = name.trim();
    if (!v || saving) return;
    saving = true;
    try {
      await onRename(v);
    } catch {
      // The page holds the reason and shows it; what you typed stays put.
    }
    saving = false;
  }
</script>

<div class="pt-48 pb-16">
  <Panel>
    <p class="font-display flex items-center gap-2 text-stone-600">
      <a href="/kitchens" class="text-stone-500 underline">Kitchens</a>
      <span aria-hidden="true">·</span>
      Rename
    </p>

    <form class="mt-6 flex flex-col gap-3" onsubmit={submit}>
      <label class="text-xs text-stone-500" for="kitchen-name">
        What do you call it?
      </label>
      <input
        id="kitchen-name"
        bind:value={name}
        class="rounded-card border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
      />

      {#if error}
        <p role="alert" class="text-sm text-paprika-500">{error}</p>
      {/if}

      <div>
        <Button type="submit" disabled={saving} dot="cocoa">
        {saving ? "Renaming…" : "Save name"}
        </Button>
      </div>
    </form>
  </Panel>
</div>
