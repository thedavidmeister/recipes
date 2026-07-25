import { cubicIn, cubicOut } from "svelte/easing";
import type { TransitionConfig } from "svelte/transition";

/** Where a page leaves to, or arrives from. */
const DIRECTIONS = ["left", "right", "up", "down"] as const;

/**
 * A page sliding on or off, in a random direction picked fresh each time — so a page
 * can leave to the left while the next drops in from the top.
 *
 * This animates a real element with a plain `transform`, deliberately **not** the View
 * Transitions API. That API runs the motion in a pseudo-element tree Safari does not
 * translate reliably, so every navigation there collapsed to a bare opacity fade — the
 * page appearing to blink out rather than travel. A transform on the element itself is
 * read the same way in every engine.
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

  const d = DIRECTIONS[Math.floor(Math.random() * DIRECTIONS.length)];
  const axis = d === "left" || d === "right" ? "X" : "Y";
  const sign = d === "left" || d === "up" ? -1 : 1;

  return {
    duration: kind === "in" ? 300 : 220,
    easing: kind === "in" ? cubicOut : cubicIn,
    // `t` runs 0→1 as the page arrives and 1→0 as it leaves, so one line does both:
    // in place and opaque at t=1, a screen away and transparent at t=0.
    css: (t) =>
      `transform: translate${axis}(${(1 - t) * sign * 100}%); opacity: ${t};`,
  };
}
