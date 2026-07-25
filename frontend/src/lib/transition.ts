import { cubicIn, cubicOut } from "svelte/easing";
import type { TransitionConfig } from "svelte/transition";

/**
 * A page sliding on or off, in a random direction picked fresh each time — so a page
 * can leave to the left while the next arrives from the right.
 *
 * This animates a real element with a plain `transform`, deliberately **not** the View
 * Transitions API. That API runs the motion in a pseudo-element tree Safari does not
 * translate reliably, so every navigation there collapsed to a bare opacity fade — the
 * page appearing to blink out rather than travel. A transform on the element itself is
 * read the same way in every engine.
 *
 * The distance is `100%` — the element's own width — which is exactly the width of the
 * clip window it sits in (both are the content column, see the `overflow-x-clip` grid
 * in the layout). So the page clears its window precisely on every viewport, rather
 * than travelling a fixed pixel count that is too short on a wide screen and too long
 * on a narrow one, and the clip keeps the off-screen half from growing a scrollbar.
 *
 * Left and right only. The clip window and the element share a width but not a height
 * (a tall page is taller than a short one), so a vertical slide could not clear cleanly
 * the same way; horizontal is uniform and exact.
 *
 * What slides is the page. The backdrop and the floating controls are rendered outside
 * the element this is applied to, so they stay put while the page moves over them —
 * the room is still, the page travels.
 *
 * Reduced motion is answered with an instant, motionless swap.
 */
export function pageSlide(
  _node: Element,
  { kind }: { kind: "in" | "out" },
): TransitionConfig {
  const reduce =
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (reduce) return { duration: 0 };

  const sign = Math.random() < 0.5 ? -1 : 1;

  return {
    duration: kind === "in" ? 300 : 220,
    easing: kind === "in" ? cubicOut : cubicIn,
    // `t` runs 0→1 as the page arrives and 1→0 as it leaves, so one line does both:
    // in place and opaque at t=1, a column-width away and transparent at t=0.
    css: (t) => `transform: translateX(${(1 - t) * sign * 100}%); opacity: ${t};`,
  };
}
