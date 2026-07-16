<script lang="ts">
  import type { Section } from "$lib/types";

  /**
   * The whole app in four words: **pick · buy · cook · joy** — the arc of a
   * meal, drawn as stops on a line to a destination.
   *
   * It is a route, not a tab bar, and that is the point: the four are *ordered*,
   * and they go somewhere. `joy` is the destination, so the line always shows how
   * far along you are and what is still ahead. A tab bar would say these are four
   * equal places you might visit; this says you are on your way to dinner.
   *
   * Behind you is solid, ahead is faint. You can still jump to any stop — the
   * line describes the journey, it does not police it.
   */
  interface Props {
    current: Section;
  }

  let { current }: Props = $props();

  const stops: { id: Section; label: string }[] = [
    { id: "pick", label: "pick" },
    { id: "buy", label: "buy" },
    { id: "cook", label: "cook" },
    { id: "joy", label: "joy" },
  ];

  const index = $derived(
    Math.max(
      0,
      stops.findIndex((s) => s.id === current),
    ),
  );

  // The line runs between the first and last dot, not edge to edge: with four
  // equal columns the outer dots sit an eighth in from each side, so the track
  // spans 12.5% → 87.5% and the travelled part is a fraction of that 75%.
  const TRACK_LEFT = 12.5;
  const TRACK_WIDTH = 75;
  const travelled = $derived((index / (stops.length - 1)) * TRACK_WIDTH);
</script>

<nav
  aria-label="Sections"
  class="sticky top-0 z-10 border-b border-neutral-200 bg-white/95 pt-4 pb-3 backdrop-blur"
>
  <ol class="relative mx-auto flex max-w-md">
    <!-- The line ahead. -->
    <div
      class="absolute top-[7px] h-0.5 bg-neutral-200"
      style="left: {TRACK_LEFT}%; width: {TRACK_WIDTH}%"
      aria-hidden="true"
    ></div>
    <!-- The line behind, drawn over it. -->
    <div
      class="absolute top-[7px] h-0.5 bg-neutral-900 transition-[width] duration-300"
      style="left: {TRACK_LEFT}%; width: {travelled}%"
      aria-hidden="true"
    ></div>

    {#each stops as stop, i (stop.id)}
      {@const passed = i < index}
      {@const here = i === index}
      <li class="relative flex-1">
        <a
          href="/{stop.id}"
          aria-current={here ? "page" : undefined}
          class="group flex flex-col items-center gap-2"
        >
          <span
            class="size-4 rounded-full border-2 transition-colors {here
              ? 'border-neutral-900 bg-white ring-4 ring-neutral-900/10'
              : passed
                ? 'border-neutral-900 bg-neutral-900'
                : 'border-neutral-300 bg-white group-hover:border-neutral-400'}"
            aria-hidden="true"
          ></span>
          <span
            class="text-sm transition-colors {here
              ? 'font-semibold text-neutral-900'
              : passed
                ? 'text-neutral-600'
                : 'text-neutral-400 group-hover:text-neutral-600'}"
          >
            {stop.label}
          </span>
        </a>
      </li>
    {/each}
  </ol>
</nav>
