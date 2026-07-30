<script lang="ts">
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  /**
   * Making a kitchen (#72) — a page, because it is a thing you are doing rather than a
   * corner of the list you are looking at.
   *
   * Naming it is the whole of it. Everything else about a kitchen — who is in it, what
   * is in it — is added once it exists, from the kitchen itself.
   *
   * It opens with its own title and no link above it (#119). The old "Kitchens ·" prefix
   * drew a breadcrumb trail, and a trail through the list is the one shape this app must
   * not have: kitchens are not a tree you descend, they are the room you are already in.
   * The way out is the `kitchens` link in the account row, which every page carries.
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
    <h1 class="font-display text-2xl font-medium text-stone-900">
      New kitchen
    </h1>

    <form class="mt-6 flex flex-col gap-3" onsubmit={create}>
      <label class="text-xs text-stone-500" for="kitchen-name">
        What do you call it?
      </label>
      <input
        id="kitchen-name"
        bind:value={name}
        placeholder="Home"
        class="rounded-card bg-cream-100 border border-stone-200 px-4 py-3 text-stone-900"
      />

      {#if error}
        <p role="alert" class="text-paprika-500 text-sm">{error}</p>
      {/if}

      <div>
        <Button type="submit" disabled={saving} dot="cocoa">
          {saving ? "Creating…" : "Create kitchen"}
        </Button>
      </div>
    </form>
  </Panel>
</div>
