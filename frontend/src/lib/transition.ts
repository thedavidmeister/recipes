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
 * The distance is `100%` of the element's width, and the element spans the full
 * viewport (the content column sits centred inside it, see the layout), so the page
 * travels a whole screen and leaves it rather than only crossing its own narrow column.
 * `overflow-x-clip` on that full-width element keeps the off-screen half from growing a
 * scrollbar.
 *
 * Left and right only: a horizontal slide clears the viewport cleanly, while a vertical
 * one would have to travel the page's height, which varies, so it could not clear the
 * same way.
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
    // in place and opaque at t=1, a screen away and transparent at t=0.
    css: (t) => `transform: translateX(${(1 - t) * sign * 100}%); opacity: ${t};`,
  };
}
