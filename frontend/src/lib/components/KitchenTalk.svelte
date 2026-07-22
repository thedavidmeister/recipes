<script lang="ts">
  /**
   * Two mice in a kitchen, mid-conversation. The pot is on and half-ignored, the
   * board is abandoned half-chopped, and both of them are turned to each other
   * instead — the cooking is the excuse, the evening is the point.
   *
   * Line art in warm ink on cream, with the meal-arc colours (#36) held to the food
   * and the glasses. Inline SVG — no image request, deterministic for the visual
   * fence, and every colour arrives through `currentColor` on a palette text
   * utility rather than a raw value.
   */

  /** A mouse, mirrored by `dir`: -1 faces left, 1 faces right. */
  const MICE = [
    { x: 146, y: 58, dir: 1 },
    { x: 262, y: 55, dir: -1 },
  ];
</script>

<svg viewBox="0 0 400 180" aria-hidden="true" class="w-full text-cocoa-500">
  <g
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <!-- A shelf, off to the side of the pair. -->
    <path d="M300 46h84" />
    <g class="text-cream-50">
      <rect x="308" y="28" width="15" height="18" rx="3" fill="currentColor" />
      <rect x="330" y="24" width="12" height="22" rx="3" fill="currentColor" />
      <rect x="350" y="31" width="17" height="15" rx="3" fill="currentColor" />
    </g>
    <rect x="308" y="28" width="15" height="18" rx="3" />
    <rect x="330" y="24" width="12" height="22" rx="3" />
    <rect x="350" y="31" width="17" height="15" rx="3" />

    {#each MICE as m (m.x)}
      <!-- Body, shoulders up out of the counter. -->
      <g class="text-cream-50">
        <path
          d="M{m.x - 26} 122c-2-26 8-44 26-44s28 18 26 44z"
          fill="currentColor"
        />
      </g>
      <path d="M{m.x - 26} 122c-2-26 8-44 26-44s28 18 26 44" />

      <!-- Ears: the whole reason a mouse reads as a mouse. -->
      <g class="text-cream-50">
        <circle cx={m.x - 14} cy={m.y - 16} r="10" fill="currentColor" />
        <circle cx={m.x + 14} cy={m.y - 16} r="10" fill="currentColor" />
        <circle cx={m.x} cy={m.y} r="17" fill="currentColor" />
      </g>
      <circle cx={m.x - 14} cy={m.y - 16} r="10" />
      <circle cx={m.x + 14} cy={m.y - 16} r="10" />
      <circle cx={m.x - 14} cy={m.y - 16} r="5" />
      <circle cx={m.x + 14} cy={m.y - 16} r="5" />
      <circle cx={m.x} cy={m.y} r="17" />

      <!-- Snout turned toward the other one — that's the conversation. -->
      <g class="text-cream-50">
        <ellipse
          cx={m.x + m.dir * 15}
          cy={m.y + 6}
          rx="9"
          ry="6.5"
          fill="currentColor"
        />
      </g>
      <ellipse cx={m.x + m.dir * 15} cy={m.y + 6} rx="9" ry="6.5" />
      <circle cx={m.x + m.dir * 23} cy={m.y + 5} r="2.2" />
      <!-- An eye, and whiskers. -->
      <circle cx={m.x + m.dir * 5} cy={m.y - 3} r="2" />
      <path
        d="M{m.x + m.dir * 22} {m.y + 9}l{m.dir * 10} 4M{m.x + m.dir * 22} {m.y +
          11}l{m.dir * 9} 8"
      />
    {/each}

    <!-- The counter they're both leaning on. -->
    <g class="text-cream-100">
      <rect x="16" y="122" width="368" height="12" rx="4" fill="currentColor" />
      <rect x="24" y="134" width="352" height="46" fill="currentColor" />
    </g>
    <rect x="16" y="122" width="368" height="12" rx="4" />
    <path d="M24 134v46M152 134v46M280 134v46M376 134v46" />
    <path d="M78 152h24M204 152h24M320 152h24" />

    <!-- The pot: on, steaming, unwatched, and clear of them both. -->
    <path d="M194 96c-2-6 0-10 4-12M204 92c-3-7 0-11 4-13" />
    <g class="text-pesto-100">
      <rect x="184" y="104" width="38" height="18" rx="5" fill="currentColor" />
    </g>
    <rect x="184" y="104" width="38" height="18" rx="5" />
    <path d="M180 108h4M222 108h4" />

    <!-- The board, chopped halfway and left: a handle hole and a few slices, so it
         reads as a board rather than a row of dials. -->
    <g class="text-cream-50">
      <rect x="44" y="108" width="62" height="14" rx="4" fill="currentColor" />
    </g>
    <rect x="44" y="108" width="62" height="14" rx="4" />
    <circle cx="51" cy="115" r="2.5" />
    <g class="text-paprika-100">
      <ellipse cx="70" cy="115" rx="5" ry="4" fill="currentColor" />
      <ellipse cx="82" cy="115" rx="5" ry="4" fill="currentColor" />
      <ellipse cx="94" cy="115" rx="5" ry="4" fill="currentColor" />
    </g>
    <ellipse cx="70" cy="115" rx="5" ry="4" />
    <ellipse cx="82" cy="115" rx="5" ry="4" />
    <ellipse cx="94" cy="115" rx="5" ry="4" />

    <!-- Two glasses, the actual point of the evening. -->
    <g class="text-honey-100">
      <path d="M116 106h14l-2 12h-10z" fill="currentColor" />
      <path d="M302 104h14l-2 14h-10z" fill="currentColor" />
    </g>
    <path d="M116 106h14l-2 12h-10zM123 118v4M118 122h10" />
    <path d="M302 104h14l-2 14h-10zM309 118v4M304 122h10" />
  </g>
</svg>
