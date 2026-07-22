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
   */
</script>

<div aria-hidden="true" class="page-backdrop fixed inset-0 -z-10">
  <img src="/pick.jpg" alt="" class="size-full object-cover" />
</div>
