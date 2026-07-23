<script lang="ts">
  /**
   * The shape of something that has not arrived.
   *
   * Deliberately a shape and not a spinner: it says *what* is coming and roughly how
   * much of it, so the page does not jump when it lands. No margin of its own — where
   * it sits is the caller's business, and a component that decides its own spacing is
   * one you end up fighting.
   */
  let { rounded = "card" }: { rounded?: "card" | "pill" } = $props();
</script>

<div
  class="h-10 w-full bg-stone-100 {rounded === 'pill'
    ? 'rounded-pill'
    : 'rounded-card'}"
  aria-hidden="true"
></div>
