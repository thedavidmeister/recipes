<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * The solid surface everything readable sits on.
   *
   * It is what makes the photographs behind the kitchens and the pick work: the room
   * runs at full strength and the words sit on their own ground, rather than the
   * picture being washed out to make them legible. Eight pages were repeating its
   * class list, which is eight chances for one of them to be slightly different.
   */
  let { children }: { children: Snippet } = $props();
</script>

<div class="rounded-card bg-cream-50 p-6">
  {@render children()}
</div>
