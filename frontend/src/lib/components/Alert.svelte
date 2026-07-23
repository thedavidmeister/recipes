<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * Something went wrong, said in paprika.
   *
   * The one place the palette is allowed to raise its voice, which only works because
   * nothing else does — a page of alarming colours says nothing is alarming.
   */
  let { children }: { children: Snippet } = $props();
</script>

<div class="rounded-card border-paprika-500/30 bg-paprika-100 border p-6">
  {@render children()}
</div>
