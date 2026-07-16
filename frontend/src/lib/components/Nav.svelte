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
   *
   * The stop you are on has a colour (pick·pesto, buy·plum, cook·paprika,
   * joy·honey), and the whole trail behind you — the line and the dots you have
   * passed — takes that same colour, so where you are reads as one clean run
   * rather than a rainbow. Ahead stays a faint grey. The classes are spelled out
   * per stop, not built from the id, so Tailwind actually generates them.
   */
  interface Props {
    current: Section;
  }

  let { current }: Props = $props();

  const stops: {
    id: Section;
    label: string;
    ring: string;
    line: string;
    dot: string;
  }[] = [
    { id: "pick", label: "pick", ring: "border-pesto-500 ring-pesto-500/20", line: "bg-pesto-500", dot: "border-pesto-500 bg-pesto-500" },
    { id: "buy", label: "buy", ring: "border-plum-500 ring-plum-500/20", line: "bg-plum-500", dot: "border-plum-500 bg-plum-500" },
    { id: "cook", label: "cook", ring: "border-paprika-500 ring-paprika-500/20", line: "bg-paprika-500", dot: "border-paprika-500 bg-paprika-500" },
    { id: "joy", label: "joy", ring: "border-honey-500 ring-honey-500/20", line: "bg-honey-500", dot: "border-honey-500 bg-honey-500" },
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
  class="bg-cream-50/95 font-display sticky top-0 z-10 border-b border-stone-200 pt-4 pb-3 backdrop-blur"
>
  <ol class="relative mx-auto flex max-w-md">
    <!-- The line ahead. -->
    <div
      class="absolute top-[7px] h-0.5 bg-stone-200"
      style="left: {TRACK_LEFT}%; width: {TRACK_WIDTH}%"
      aria-hidden="true"
    ></div>
    <!-- The line behind: a single bar up to the current stop, in that stop's
         colour, so the whole trail matches the dot you are on. -->
    {#if index > 0}
      <div
        class="absolute top-[7px] h-0.5 transition-[width] duration-300 {stops[index].line}"
        style="left: {TRACK_LEFT}%; width: {travelled}%"
        aria-hidden="true"
      ></div>
    {/if}

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
              ? 'bg-cream-50 ring-4 ' + stop.ring
              : passed
                ? stops[index].dot
                : 'bg-cream-50 border-stone-300 group-hover:border-stone-400'}"
            aria-hidden="true"
          ></span>
          <span
            class="text-sm transition-colors {here
              ? 'font-semibold text-stone-900'
              : passed
                ? 'text-stone-600'
                : 'text-stone-400 group-hover:text-stone-600'}"
          >
            {stop.label}
          </span>
        </a>
      </li>
    {/each}
  </ol>
</nav>
