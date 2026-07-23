<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * A centred card for a page that is waiting, empty, or explaining itself — anything
   * that is a state rather than content.
   *
   * Bordered and on cream rather than the plain panel, so "nothing here yet" reads as
   * a deliberate message instead of a page that failed to fill.
   */
  let { children }: { children: Snippet } = $props();
</script>

<div class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center">
  {@render children()}
</div>
