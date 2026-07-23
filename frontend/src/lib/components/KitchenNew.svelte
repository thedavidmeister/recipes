<script lang="ts">
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  /**
   * Making a kitchen (#72) — a page, because it is a thing you are doing rather than a
   * corner of the list you are looking at.
   *
   * Naming it is the whole of it. Everything else about a kitchen — who is in it, what
   * is in it — is added once it exists, from the kitchen itself.
   */
  interface Props {
    error?: string;
    onCreate: (name: string) => void | Promise<void>;
  }

  let { error, onCreate }: Props = $props();

  let name = $state("");
  let saving = $state(false);

  async function create(e: Event) {
    e.preventDefault();
    const v = name.trim();
    if (!v || saving) return;
    saving = true;
    try {
      await onCreate(v);
    } catch {
      // The page has the reason and shows it; what you typed stays put so the retry
      // is one press rather than a retype.
    }
    saving = false;
  }
</script>

<div class="pt-48 pb-16">
  <Panel>
    <p class="font-display flex items-center gap-2 text-stone-600">
      <a href="/kitchens" class="text-stone-500 underline">Kitchens</a>
      <span aria-hidden="true">·</span>
      New
    </p>

    <form class="mt-6 flex flex-col gap-3" onsubmit={create}>
      <label class="text-xs text-stone-500" for="kitchen-name">
        What do you call it?
      </label>
      <input
        id="kitchen-name"
        bind:value={name}
        placeholder="Home"
        class="rounded-card border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
      />

      {#if error}
        <p role="alert" class="text-sm text-paprika-500">{error}</p>
      {/if}

      <div>
        <Button type="submit" disabled={saving} dot="cocoa">
        {saving ? "Making it…" : "Create kitchen"}
        </Button>
      </div>
    </form>
  </Panel>
</div>
