<script lang="ts">
  /**
   * The kitchen photograph behind the kitchens routes (#87), fixed so the content
   * scrolls over it.
   *
   * The photograph runs at full strength, unwashed. Legibility is not its problem:
   * everything readable sits on its own solid panel, so nothing has to be faded out to
   * make room for text.
   *
   * Its own component so the pages and the story that checks legibility over it render
   * the *same* markup — a lookalike copy in a story would drift from the page and
   * quietly stop telling the truth.
   *
   * `/kitchen.jpg` is 1800x3200 — exactly twice the visual fence's 900x800 viewport on
   * the axis `object-cover` scales by, so at the fence's device scale of 2 the
   * photograph draws one source pixel per device pixel with nothing to resample. Under
   * a fractional scale the browser was free to resolve the image's last column either
   * of two ways, and two shoots of one build disagreed about it (#223). `visual-shoot`
   * asserts the 1:1 on every shot, so a re-crop that breaks it fails loudly.
   */
</script>

<div aria-hidden="true" class="fixed inset-0 -z-10">
  <img src="/kitchen.jpg" alt="" class="size-full object-cover" />
</div>
