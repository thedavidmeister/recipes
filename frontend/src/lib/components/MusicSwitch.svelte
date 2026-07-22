<script lang="ts">
  /**
   * The way out of the music that the splash promised (#88).
   *
   * It renders the switch and nothing else: the app layout owns the audio element and
   * the remembered preference, the same split as a page owning its query while the
   * component owns rendering.
   */
  let { playing, onToggle }: { playing: boolean; onToggle: () => void } =
    $props();
</script>

<button
  type="button"
  onclick={onToggle}
  aria-pressed={playing}
  class="rounded-pill fixed bottom-6 right-6 z-20 flex items-center gap-2 px-4 py-3 text-sm font-medium
  {playing
    ? 'bg-cocoa-500 text-cream-50'
    : 'bg-cream-50 text-espresso-800 ring-cream-300 ring-1'}"
>
  <svg
    aria-hidden="true"
    class="size-5 flex-none"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <!-- The speaker is the same in both states; only the sound leaving it changes. -->
    <path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4z" />
    {#if playing}
      <path d="M15.5 9.2a4 4 0 0 1 0 5.6" />
      <path d="M18 6.8a7.5 7.5 0 0 1 0 10.4" />
    {:else}
      <path d="M16 10l4 4" />
      <path d="M20 10l-4 4" />
    {/if}
  </svg>
  {playing ? "Stop music" : "Play music"}
</button>
