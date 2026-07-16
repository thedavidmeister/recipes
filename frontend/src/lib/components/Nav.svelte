<script lang="ts">
  import type { Section } from "$lib/types";

  /**
   * The whole app in four words: **pick · buy · cook · joy** — the arc of a
   * meal, so the nav answers "what am I doing right now" rather than listing
   * features.
   *
   * A bottom bar because this is used in a kitchen, on a phone, one-thumbed.
   */
  interface Props {
    current: Section;
  }

  let { current }: Props = $props();

  const sections: { id: Section; label: string }[] = [
    { id: "pick", label: "pick" },
    { id: "buy", label: "buy" },
    { id: "cook", label: "cook" },
    { id: "joy", label: "joy" },
  ];
</script>

<nav
  aria-label="Sections"
  class="fixed inset-x-0 bottom-0 z-10 border-t border-neutral-200 bg-white/95 backdrop-blur"
>
  <ul class="mx-auto flex max-w-2xl">
    {#each sections as section (section.id)}
      <li class="flex-1">
        <a
          href="/{section.id}"
          aria-current={current === section.id ? "page" : undefined}
          class="flex flex-col items-center gap-1 py-3 text-sm font-medium transition-colors
            {current === section.id
            ? 'text-neutral-900'
            : 'text-neutral-400 hover:text-neutral-600'}"
        >
          <span
            class="h-1 w-6 rounded-full {current === section.id
              ? 'bg-neutral-900'
              : 'bg-transparent'}"
          ></span>
          {section.label}
        </a>
      </li>
    {/each}
  </ul>
</nav>
