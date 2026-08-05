<script lang="ts">
  /**
   * The photograph behind the pick (#94), fixed so the swipe scrolls over it.
   *
   * Shown at full strength, unwashed: the readable parts of the page sit on their own
   * solid panel, so nothing has to be faded out to make room for text. This is the
   * background people look at longest — pick is where you linger — so it is worth
   * being a picture rather than a texture.
   *
   * Its own component so the page and the story that checks legibility over it render
   * the *same* markup.
   *
   * The photograph ships as a ladder of widths, every rung the same 9:16 frame from one
   * master, so they compose identically and only the resolution differs — resolution
   * switching, which is what `srcset` + `sizes` is for. `sizes` is `100vw` because that
   * is literally the layout: the wrapper is `fixed inset-0`, so the image is exactly as
   * wide as the viewport and the browser selects on real device width.
   *
   * The 1800x3200 rung is the one the visual fence resolves, the same way every run:
   * the fence pins the viewport to 900 CSS px and the device scale to 2, so `100vw` is
   * exactly 900, the density asked for is exactly 2.0, and 1800w is the narrowest
   * candidate meeting it. At that rung the photograph draws one source pixel per device
   * pixel with nothing left to resample — which matters because under a fractional
   * scale the browser is free to resolve the image's last column either of two ways,
   * and two shoots of one build disagreed about it (#223). `visual-shoot` re-checks the
   * *selected* variant on every shot, so a selection that landed on another rung fails
   * the shoot loudly instead of making the fence noisy.
   */
</script>

<div aria-hidden="true" class="fixed inset-0 -z-10">
  <img
    src="/pick-1080.jpg"
    srcset="/pick-720.jpg 720w, /pick-1080.jpg 1080w, /pick-1440.jpg 1440w, /pick-1800.jpg 1800w"
    sizes="100vw"
    alt=""
    class="size-full object-cover"
  />
</div>
