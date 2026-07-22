<script lang="ts">
  /**
   * The kitchen photograph behind the kitchens page (#87), fixed so the content
   * scrolls over it.
   *
   * The scrim is a gradient rather than a flat wash, and it ramps deliberately fast:
   * the top band stays almost clear so the photograph actually reads, then it goes to
   * near-solid cream *before* any text lands. A slow fade across the whole viewport
   * put the page header over the brightest part of the photo, which was unreadable.
   *
   * Its own component so the page and the story that checks legibility over it render
   * the *same* markup — a lookalike copy in a story would drift from the page and
   * quietly stop telling the truth.
   */
</script>

<div aria-hidden="true" class="fixed inset-0 -z-10">
  <img src="/kitchen.jpg" alt="" class="size-full object-cover" />
  <div
    class="absolute inset-0 bg-gradient-to-b from-cream-50/10 from-15% via-cream-50/92 via-34% to-cream-50 to-55%"
  ></div>
</div>
