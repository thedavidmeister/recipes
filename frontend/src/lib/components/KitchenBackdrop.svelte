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
   */
</script>

<div aria-hidden="true" class="page-backdrop fixed inset-0 -z-10">
  <img src="/kitchen.jpg" alt="" class="size-full object-cover" />
</div>
