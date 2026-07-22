<script lang="ts">
  /**
   * The way out of the music, and the way to take the edge off it (#88).
   *
   * The level only appears while something is playing: a volume control over silence
   * adjusts nothing, and asking someone to set a level before they have heard it is
   * asking them to guess.
   *
   * It renders the controls and nothing else — the app layout owns the audio element,
   * the fade, and the remembered settings, the same split as a page owning its query
   * while the component owns rendering.
   */
  interface Props {
    playing: boolean;
    volume: number;
    onToggle: () => void;
    onVolume: (level: number) => void;
  }

  let { playing, volume, onToggle, onVolume }: Props = $props();
</script>

<div class="app-controls fixed bottom-6 right-6 z-20 flex items-center gap-2">
  {#if playing}
    <label
      class="rounded-pill bg-cream-50 ring-cream-300 flex items-center gap-2 px-4 py-3 ring-1"
    >
      <span class="sr-only">Volume</span>
      <svg
        aria-hidden="true"
        class="text-cocoa-500 size-4 flex-none"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.75"
        stroke-linecap="round"
      >
        <!-- A wedge: quiet at the left of the track, loud at the right. -->
        <path d="M4 15.5 20 7" />
        <path d="M4 15.5h0M20 7v9.5" />
      </svg>
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={volume}
        oninput={(e) => onVolume(Number(e.currentTarget.value))}
        class="accent-cocoa-500 w-24"
      />
    </label>
  {/if}

  <button
    type="button"
    onclick={onToggle}
    aria-pressed={playing}
    class="rounded-pill flex items-center gap-2 px-4 py-3 text-sm font-medium
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
</div>
