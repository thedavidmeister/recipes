<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * A row you press to go somewhere: the app's main way of moving.
   *
   * Since each page here does one thing, most of what a page offers is *other pages* —
   * a kitchen's equipment, its pantry, its name. These rows are that, and the arrow is
   * the promise that pressing one takes you rather than changing something in place.
   */
  interface Props {
    href: string;
    /** What sits before the arrow — a count, usually, so a number is fine. */
    trailing?: string | number;
    children: Snippet;
  }

  let { href, trailing, children }: Props = $props();
</script>

<a
  {href}
  class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
>
  {@render children()}
  <span class="text-sm text-stone-400">{trailing === undefined ? "→" : `${trailing} →`}</span>
</a>
