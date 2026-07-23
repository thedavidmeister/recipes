<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * The button, as the design system declares it: *big, round, generous — colour only
   * where you act*.
   *
   * That last clause is the whole idea, and it is what a filled slab gets wrong. The
   * surface stays cream and the border does the shaping; the colour appears as a dot
   * beside the label, so a page full of actions reads as a page rather than a wall of
   * paint. A button that fills itself with brand colour and spans the whole column is
   * what every framework ships by default, which is exactly why it looks like nothing.
   *
   * It exists as a component because it had already drifted: several pages had
   * hand-rolled a full-width `bg-cocoa-500` slab, and the design lint passed every one
   * of them — it checks that colours come from tokens, not that they are composed the
   * way the system says. A shared component is the only thing that can hold a
   * composition still.
   *
   * The design system story renders *this*, so the declaration and the thing itself
   * cannot disagree.
   */
  interface Props {
    /** `primary` to act, `secondary` to choose otherwise, `quiet` to leave. */
    variant?: "primary" | "secondary" | "quiet";
    /** The dot's colour — where the action lives on the meal arc. */
    dot?: "pesto" | "cocoa" | "paprika" | "honey" | "plum";
    /** Renders an anchor when set: navigation that reads as an action. */
    href?: string;
    type?: "button" | "submit";
    disabled?: boolean;
    onclick?: () => void;
    children: Snippet;
  }

  let {
    variant = "primary",
    dot,
    href,
    type = "button",
    disabled = false,
    onclick,
    children,
  }: Props = $props();

  // Written out rather than interpolated: Tailwind reads these as text, so a class
  // built from a variable is a class that does not exist by the time it ships.
  const SHAPE =
    "inline-flex items-center gap-2 rounded-xl px-7 py-3 text-lg font-semibold transition";

  const VARIANTS = {
    primary:
      "bg-cream-50 border-2 border-stone-300 text-stone-900 hover:border-stone-400",
    secondary:
      "bg-cream-100 border border-stone-300 text-stone-700 hover:border-stone-400",
    quiet: "px-4 font-medium text-stone-500 hover:text-stone-900",
  } as const;

  const DOTS = {
    pesto: "bg-pesto-500",
    cocoa: "bg-cocoa-500",
    paprika: "bg-paprika-500",
    honey: "bg-honey-500",
    plum: "bg-plum-500",
  } as const;

  const classes = $derived(`${SHAPE} ${VARIANTS[variant]}`);
</script>

{#if href}
  <a {href} class={classes}>
    {#if dot}
      <span class="size-2.5 flex-none rounded-full {DOTS[dot]}" aria-hidden="true"
      ></span>
    {/if}
    {@render children()}
  </a>
{:else}
  <button {type} {disabled} {onclick} class={classes}>
    {#if dot}
      <span class="size-2.5 flex-none rounded-full {DOTS[dot]}" aria-hidden="true"
      ></span>
    {/if}
    {@render children()}
  </button>
{/if}
