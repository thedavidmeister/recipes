<script lang="ts">
  /**
   * Two mice in a kitchen, mid-conversation. The pot is on and half-ignored, the
   * board is abandoned half-chopped, and both of them are turned to each other
   * instead — the cooking is the excuse, the evening is the point.
   *
   * Mice rather than people on purpose: ears, snouts and whiskers are circles and
   * arcs, which line art carries well. The room is drawn dense — tiled splashback,
   * a window, a rail of hanging utensils, loaded shelves — because the page is
   * deliberately light on function and the picture is what carries it.
   *
   * Inline SVG throughout: no image request, deterministic for the visual fence, and
   * every colour arrives through `currentColor` on a palette text utility.
   */

  // --- the splashback, laid in running bond behind the counter -------------------
  const TILE_W = 24;
  const TILE_H = 11;
  const TILE_TOP = 86;
  const TILE_ROWS = 4;

  const tiles = $derived.by(() => {
    const out: { x: number; y: number }[] = [];
    for (let row = 0; row < TILE_ROWS; row++) {
      const shift = row % 2 === 1 ? TILE_W / 2 : 0;
      for (let col = -1; col <= 400 / TILE_W; col++) {
        out.push({ x: col * TILE_W + shift, y: TILE_TOP + row * TILE_H });
      }
    }
    return out;
  });

  /** Jars and bottles along a shelf: [x, width, height, tint]. */
  const SHELF_TOP: [number, number, number, string][] = [
    [300, 13, 20, "text-cream-50"],
    [318, 10, 26, "text-pesto-100"],
    [333, 16, 16, "text-cream-50"],
    [354, 11, 23, "text-honey-100"],
    [370, 14, 18, "text-cream-50"],
  ];
  const SHELF_LOW: [number, number, number, string][] = [
    [302, 17, 14, "text-plum-100"],
    [324, 12, 19, "text-cream-50"],
    [341, 15, 15, "text-paprika-100"],
    [361, 19, 12, "text-cream-50"],
  ];

  /** Hanging utensils on the rail: [x, kind]. */
  const HANGING: [number, string][] = [
    [176, "ladle"],
    [192, "whisk"],
    [208, "spatula"],
    [224, "spoon"],
  ];

  const MICE = [
    { x: 135, y: 100, dir: 1, stripes: true },
    { x: 272, y: 97, dir: -1, stripes: false },
  ];
</script>

<svg viewBox="0 0 400 200" aria-hidden="true" class="w-full text-cocoa-500">
  <!-- Splashback: glazed tiles on a slightly deeper ground. -->
  <g class="text-cream-200">
    <rect x="0" y={TILE_TOP} width="400" height={TILE_ROWS * TILE_H} fill="currentColor" />
  </g>
  <g class="text-cream-50">
    {#each tiles as t, i (i)}
      <rect x={t.x + 0.8} y={t.y + 0.8} width={TILE_W - 1.6} height={TILE_H - 1.6} rx="1.5" fill="currentColor" />
    {/each}
  </g>

  <g
    fill="none"
    stroke="currentColor"
    stroke-width="1.4"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <!-- Window, with something growing on the sill. -->
    <rect x="16" y="12" width="86" height="62" rx="4" />
    <path d="M59 12v62M16 43h86" />
    <path d="M12 74h94" />
    <g class="text-pesto-100">
      <path d="M30 74c0-9 4-14 9-14s9 5 9 14z" fill="currentColor" />
    </g>
    <path d="M30 74c0-9 4-14 9-14s9 5 9 14z" />
    <path d="M39 60v-8M39 55c-5-1-7-5-6-8 3 0 6 3 6 8M39 55c5-2 7-6 6-9-3 0-6 4-6 9" />

    <!-- Clock. -->
    <circle cx="126" cy="34" r="13" />
    <circle cx="126" cy="34" r="1.6" />
    <path d="M126 34v-7M126 34l5 3" />

    <!-- Rail of hanging utensils. -->
    <path d="M166 20h72" />
    {#each HANGING as [hx, kind] (hx)}
      <circle cx={hx} cy="20" r="2.4" />
      <path d="M{hx} 22v{kind === 'whisk' ? 14 : 16}" />
      {#if kind === 'ladle'}
        <circle cx={hx} cy="43" r="5" />
      {:else if kind === 'whisk'}
        <path d="M{hx - 5} 36c0 8 2 12 5 12s5-4 5-12M{hx} 36v12M{hx - 2} 36c-1 8 0 12 2 12M{hx + 2} 36c1 8 0 12-2 12" />
      {:else if kind === 'spatula'}
        <rect x={hx - 4.5} y="38" width="9" height="11" rx="2" />
      {:else}
        <ellipse cx={hx} cy="43" rx="4" ry="5.5" />
      {/if}
    {/each}

    <!-- Garlic and herbs, hung to dry. -->
    <path d="M256 20v8" />
    <g class="text-cream-50">
      <circle cx="256" cy="33" r="5" fill="currentColor" />
      <circle cx="251" cy="38" r="4.5" fill="currentColor" />
      <circle cx="261" cy="38" r="4.5" fill="currentColor" />
    </g>
    <circle cx="256" cy="33" r="5" />
    <circle cx="251" cy="38" r="4.5" />
    <circle cx="261" cy="38" r="4.5" />

    <!-- Open shelving, loaded. -->
    <path d="M292 48h96M292 82h96" />
    {#each SHELF_TOP as [sx, w, h, tint] (sx)}
      <g class={tint}><rect x={sx} y={48 - h} width={w} height={h} rx="2.5" fill="currentColor" /></g>
      <rect x={sx} y={48 - h} width={w} height={h} rx="2.5" />
      <path d="M{sx + 2} {48 - h + 4}h{w - 4}" />
    {/each}
    {#each SHELF_LOW as [sx, w, h, tint] (sx)}
      <g class={tint}><rect x={sx} y={82 - h} width={w} height={h} rx="2.5" fill="currentColor" /></g>
      <rect x={sx} y={82 - h} width={w} height={h} rx="2.5" />
    {/each}

    <!-- Stacked plates on the lower shelf's left. -->
    <path d="M292 78h0" />

    <!-- The two of them. -->
    {#each MICE as m (m.x)}
      <!-- Body. -->
      <g class="text-cream-50">
        <path d="M{m.x - 28} 130c-2-28 8-46 28-46s30 18 28 46z" fill="currentColor" />
      </g>
      <path d="M{m.x - 28} 130c-2-28 8-46 28-46s30 18 28 46" />

      <!-- Apron, with a neck strap and a pocket. -->
      <g class="text-honey-100">
        <path d="M{m.x - 19} 130c-1-20 4-30 19-30s20 10 19 30z" fill="currentColor" />
      </g>
      <path d="M{m.x - 19} 130c-1-20 4-30 19-30s20 10 19 30" />
      <path d="M{m.x - 11} 101l6-8M{m.x + 11} 101l-6-8" />
      <rect x={m.x - 9} y="112" width="18" height="11" rx="2" />
      {#if m.stripes}
        <path d="M{m.x - 17} 108h34M{m.x - 18} 116h9M{m.x + 9} 116h9M{m.x - 18} 124h36" />
      {/if}

      <!-- Ears. -->
      <g class="text-cream-50">
        <circle cx={m.x - 15} cy={m.y - 18} r="11" fill="currentColor" />
        <circle cx={m.x + 15} cy={m.y - 18} r="11" fill="currentColor" />
        <circle cx={m.x} cy={m.y} r="19" fill="currentColor" />
      </g>
      <circle cx={m.x - 15} cy={m.y - 18} r="11" />
      <circle cx={m.x + 15} cy={m.y - 18} r="11" />
      <g class="text-plum-100">
        <circle cx={m.x - 15} cy={m.y - 18} r="5.5" fill="currentColor" />
        <circle cx={m.x + 15} cy={m.y - 18} r="5.5" fill="currentColor" />
      </g>
      <circle cx={m.x - 15} cy={m.y - 18} r="5.5" />
      <circle cx={m.x + 15} cy={m.y - 18} r="5.5" />
      <circle cx={m.x} cy={m.y} r="19" />

      <!-- Snout turned to the other one, with a nose, a smile and whiskers. -->
      <g class="text-cream-50">
        <ellipse cx={m.x + m.dir * 17} cy={m.y + 7} rx="10" ry="7.5" fill="currentColor" />
      </g>
      <ellipse cx={m.x + m.dir * 17} cy={m.y + 7} rx="10" ry="7.5" />
      <g class="text-plum-100">
        <ellipse cx={m.x + m.dir * 25} cy={m.y + 5} rx="2.8" ry="2.2" fill="currentColor" />
      </g>
      <ellipse cx={m.x + m.dir * 25} cy={m.y + 5} rx="2.8" ry="2.2" />
      <path d="M{m.x + m.dir * 25} {m.y + 8}v3c0 2 {m.dir * -3} 3 {m.dir * -5} 1" />
      <circle cx={m.x + m.dir * 6} cy={m.y - 4} r="2.6" />
      <path
        d="M{m.x + m.dir * 24} {m.y + 10}l{m.dir * 11} 3M{m.x + m.dir * 24} {m.y + 12}l{m.dir * 10} 7M{m.x + m.dir * 23} {m.y + 13}l{m.dir * 7} 9"
      />
    {/each}

    <!-- The counter. -->
    <g class="text-cream-100">
      <rect x="0" y="130" width="400" height="12" rx="3" fill="currentColor" />
      <rect x="6" y="142" width="388" height="58" fill="currentColor" />
    </g>
    <rect x="0" y="130" width="400" height="12" rx="3" />
    <path d="M6 142v58M104 142v58M202 142v58M300 142v58M394 142v58" />
    <path d="M40 158v34M138 158v34M236 158v34M334 158v34" />
    <path d="M36 160h8M134 160h8M232 160h8M330 160h8" />
    <!-- A tea towel over a handle. -->
    <g class="text-plum-100">
      <path d="M244 160h14v26c0 3-14 3-14 0z" fill="currentColor" />
    </g>
    <path d="M244 160h14v26c0 3-14 3-14 0zM248 164v18M254 164v18" />

    <!-- The pot: on, steaming, unwatched. -->
    <path d="M190 108c-2-7 0-11 4-13M200 104c-3-8 0-12 4-14" />
    <g class="text-pesto-100">
      <rect x="180" y="114" width="40" height="16" rx="5" fill="currentColor" />
    </g>
    <rect x="180" y="114" width="40" height="16" rx="5" />
    <path d="M178 112h44" />
    <path d="M174 118h6M220 118h6" />
    <circle cx="200" cy="109" r="2" />

    <!-- Board, chopped halfway: a knife set down, slices, an onion, herbs. -->
    <g class="text-cream-50">
      <rect x="42" y="116" width="60" height="14" rx="4" fill="currentColor" />
    </g>
    <rect x="42" y="116" width="60" height="14" rx="4" />
    <circle cx="49" cy="123" r="2.5" />
    <g class="text-paprika-100">
      <ellipse cx="66" cy="123" rx="5" ry="4" fill="currentColor" />
      <ellipse cx="78" cy="123" rx="5" ry="4" fill="currentColor" />
      <ellipse cx="90" cy="123" rx="5" ry="4" fill="currentColor" />
    </g>
    <ellipse cx="66" cy="123" rx="5" ry="4" />
    <ellipse cx="78" cy="123" rx="5" ry="4" />
    <ellipse cx="90" cy="123" rx="5" ry="4" />
    <path d="M62 123h8M74 123h8M86 123h8" />

    <!-- A bowl of something, and a stack of two more. -->
    <g class="text-honey-100">
      <path d="M12 118h26c0 8-6 12-13 12s-13-4-13-12z" fill="currentColor" />
    </g>
    <path d="M12 118h26c0 8-6 12-13 12s-13-4-13-12z" />
    <path d="M14 122h22" />

    <!-- A bottle someone brought. -->
    <g class="text-plum-100">
      <path d="M306 130v-16c0-4 3-5 3-8v-4h6v4c0 3 3 4 3 8v16z" fill="currentColor" />
    </g>
    <path d="M306 130v-16c0-4 3-5 3-8v-4h6v4c0 3 3 4 3 8v16" />
    <path d="M307 119h11" />

    <!-- Two glasses, the actual point of the evening. -->
    <g class="text-honey-100">
      <path d="M226 114h15l-2 13h-11z" fill="currentColor" />
      <path d="M324 112h15l-2 15h-11z" fill="currentColor" />
    </g>
    <path d="M226 114h15l-2 13h-11zM233 127v3M228 130h11" />
    <path d="M324 112h15l-2 15h-11zM331 127v3M326 130h11" />

    <!-- Salt and pepper. -->
    <g class="text-cream-50">
      <rect x="344" y="118" width="9" height="12" rx="2.5" fill="currentColor" />
      <rect x="357" y="120" width="9" height="10" rx="2.5" fill="currentColor" />
    </g>
    <rect x="344" y="118" width="9" height="12" rx="2.5" />
    <rect x="357" y="120" width="9" height="10" rx="2.5" />
    <path d="M347 121h3M360 123h3" />

    <!-- A loaf, because someone brought bread. -->
    <g class="text-cream-50">
      <path d="M370 130c-2-10 4-16 12-16s14 6 12 16z" fill="currentColor" />
    </g>
    <path d="M370 130c-2-10 4-16 12-16s14 6 12 16z" />
    <path d="M375 120l3-4M382 118v-4M389 120l-3-4" />
  </g>
</svg>
