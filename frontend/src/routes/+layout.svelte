<script lang="ts">
  import "../app.css";
  import favicon from "$lib/assets/favicon.svg";
  import { QueryClient, QueryClientProvider } from "@tanstack/svelte-query";
  import { onNavigate } from "$app/navigation";

  let { children } = $props();
  const queryClient = new QueryClient();

  /** Where a page leaves to, or arrives from: [x, y] translations. */
  const DIRECTIONS = [
    ["-100%", "0"],
    ["100%", "0"],
    ["0", "-100%"],
    ["0", "100%"],
  ] as const;

  const someDirection = () =>
    DIRECTIONS[Math.floor(Math.random() * DIRECTIONS.length)];

  /**
   * Slide between pages: the outgoing one leaves in one random direction, the
   * incoming one arrives from another. The app is a SPA, so every navigation is
   * client-side — and now that moving between views *is* the state model (a kitchen,
   * its equipment, its pantry are each their own page), the movement is the point.
   *
   * The two directions are picked independently, so a page can leave left while the
   * next drops in from the top. They're handed to CSS as custom properties because
   * the animation belongs in `app.css` — which is also where reduced motion is
   * honoured.
   *
   * Progressive: a browser without the View Transitions API simply navigates.
   */
  onNavigate((navigation) => {
    if (!document.startViewTransition) return;

    const [exitX, exitY] = someDirection();
    const [enterX, enterY] = someDirection();
    const root = document.documentElement;
    root.style.setProperty("--page-exit-x", exitX);
    root.style.setProperty("--page-exit-y", exitY);
    root.style.setProperty("--page-enter-x", enterX);
    root.style.setProperty("--page-enter-y", enterY);

    return new Promise((resolve) => {
      document.startViewTransition(async () => {
        resolve();
        await navigation.complete;
      });
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
