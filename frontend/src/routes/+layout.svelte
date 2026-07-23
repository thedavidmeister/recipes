<script lang="ts">
  import "../app.css";
  import favicon from "$lib/assets/favicon.svg";
  import { QueryClient, QueryClientProvider } from "@tanstack/svelte-query";
  import { retryTransient } from "$lib/client";
  import { onNavigate } from "$app/navigation";

  let { children } = $props();
  // One retry policy for every query in the app: patient with a server that has not
  // woken up, and unargumentative with one that has answered. See `retryTransient`.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: retryTransient } },
  });

  /** Where a page leaves to, or arrives from. */
  const DIRECTIONS = ["left", "right", "up", "down"] as const;

  const someDirection = () =>
    DIRECTIONS[Math.floor(Math.random() * DIRECTIONS.length)];

  const clearDirections = (root: HTMLElement) => {
    for (const d of DIRECTIONS) {
      root.classList.remove(`page-exit-${d}`, `page-enter-${d}`);
    }
  };

  /**
   * Slide between pages: the outgoing one leaves in one random direction, the incoming
   * one arrives from another, picked independently — so a page can leave left while the
   * next drops in from the top.
   *
   * The directions are handed to CSS as classes on the root element. They were custom
   * properties, which does not work: a view transition animates in its own
   * pseudo-element tree, and custom properties do not reliably inherit into it. Safari
   * did not, so the translation resolved to zero and every navigation was a bare fade —
   * the page appearing to blink out rather than travel. A class on the originating
   * element is read the same way everywhere.
   *
   * Reduced motion is answered here rather than in CSS. Turning the animation off still
   * swaps two snapshots, which flickers; declining the transition entirely just
   * navigates, which is what someone asking for less motion is asking for.
   *
   * Progressive: a browser without the View Transitions API simply navigates.
   */
  onNavigate((navigation) => {
    if (!document.startViewTransition) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    const root = document.documentElement;
    clearDirections(root);
    root.classList.add(
      `page-exit-${someDirection()}`,
      `page-enter-${someDirection()}`,
    );

    return new Promise((resolve) => {
      const transition = document.startViewTransition(async () => {
        resolve();
        await navigation.complete;
      });
      // Whatever happens to it — finished, skipped, interrupted — the classes must not
      // outlive it, or the next navigation inherits a direction it did not choose.
      void transition.finished.catch(() => {}).finally(() => clearDirections(root));
    });
  });
</script>

<svelte:head>
  <link rel="icon" href={favicon} />
  <title>recipes</title>
</svelte:head>

<QueryClientProvider client={queryClient}>
  {@render children()}
</QueryClientProvider>
