<script lang="ts">
  import KitchenBackdrop from "./KitchenBackdrop.svelte";

  /**
   * The door into the app (#88).
   *
   * It exists for a reason more practical than ceremony: a browser will not let a
   * page make sound until someone has interacted with it, so *something* has to be
   * tapped before the music can start. Rather than hide that behind a switch on every
   * page, the tap becomes the way in — one deliberate gesture that opens the room and
   * starts the track together.
   *
   * So it says what it will do before it does it. Sound that arrives unannounced is
   * the thing people resent, and the way out is named in the same breath.
   *
   * The words sit on a panel rather than straight on the photograph, which is how
   * every readable surface in the app works — the room stays a room and the type stays
   * type.
   */
  let { onStart }: { onStart: () => void } = $props();
</script>

<KitchenBackdrop />

<div class="flex min-h-screen items-center justify-center px-6">
  <div class="rounded-card bg-cream-50 px-12 py-14 text-center">
    <h1 class="text-espresso-900 text-4xl font-semibold">recipes</h1>
    <p class="text-cocoa-500 mt-2 text-lg">pick · buy · cook · joy</p>

    <button
      type="button"
      onclick={onStart}
      class="rounded-pill bg-cocoa-500 text-cream-50 mt-10 px-10 py-4 text-lg font-medium"
    >
      Start
    </button>

    <p class="text-stone-500 mt-4 text-sm">
      Starts with music. Switch it off any time.
    </p>
  </div>
</div>
