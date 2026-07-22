<script lang="ts">
  /**
   * Late in the evening, two mice in a kitchen, close together under the lamp. The
   * pot is still on and neither of them is watching it — the cooking was the excuse,
   * the talking is what they're actually here for.
   *
   * Tonal, not line art: no outlines anywhere. Depth is built from filled shapes on
   * the warm ramp (oat → latte → caramel → cocoa → coffee → bean → espresso), with
   * the lamp pooling honey light over the pair and the edges of the room falling
   * away into shadow. The rim of light down their lamp-facing side is a lighter
   * shape sitting slightly proud of the darker one.
   *
   * The whole swatch is in here: the browns and creams build the room, and every
   * flavour colour turns up where it belongs — as the food. A shelf of preserves, a
   * bowl of fruit, chillies and herbs on the board, a bottle, the pot.
   *
   * Inline SVG — no image request, deterministic for the visual fence, and every
   * colour arrives through `currentColor` on a palette text utility.
   */

  /** Concentric pools make the lamp glow without needing a gradient. */
  const GLOW = [
    { rx: 236, ry: 170, o: 0.09 },
    { rx: 198, ry: 144, o: 0.1 },
    { rx: 160, ry: 118, o: 0.12 },
    { rx: 122, ry: 92, o: 0.13 },
    { rx: 86, ry: 66, o: 0.15 },
    { rx: 52, ry: 42, o: 0.18 },
  ];

  /** The preserve shelf — every flavour in the swatch, put up in jars.
   *  [x, width, height, contents, lid] */
  const JARS: [number, number, number, string, string][] = [
    [300, 15, 26, "text-chilli-500", "text-chilli-100"],
    [320, 13, 31, "text-citrus-500", "text-citrus-100"],
    [337, 17, 22, "text-orchard-500", "text-orchard-100"],
    [358, 14, 28, "text-allium-500", "text-allium-100"],
    [376, 16, 24, "text-herb-500", "text-herb-100"],
  ];
  const JARS_LOW: [number, number, number, string, string][] = [
    [302, 17, 21, "text-sea-500", "text-sea-100"],
    [323, 14, 26, "text-berry-500", "text-berry-100"],
    [341, 18, 18, "text-floral-500", "text-floral-100"],
    [363, 15, 24, "text-beet-500", "text-beet-100"],
    [382, 14, 20, "text-spice-500", "text-spice-100"],
  ];

  /** Each of them: centre, head height, and which way they're turned. */
  const MICE = [
    { x: 152, y: 150, dir: 1 },
    { x: 250, y: 144, dir: -1 },
  ];
</script>

<svg viewBox="0 0 400 300" aria-hidden="true" class="block w-full">
  <!-- The room, after dark. -->
  <g class="text-espresso-800">
    <rect x="0" y="0" width="400" height="300" fill="currentColor" />
  </g>

  <!-- Night through the window, and a moon. -->
  <g class="text-espresso-900">
    <rect x="20" y="34" width="76" height="64" rx="3" fill="currentColor" />
  </g>
  <g class="text-sea-500">
    <rect x="23" y="37" width="70" height="58" rx="2" fill="currentColor" fill-opacity="0.3" />
  </g>
  <g class="text-oat-200">
    <circle cx="76" cy="54" r="9" fill="currentColor" fill-opacity="0.65" />
    <circle cx="38" cy="50" r="1.6" fill="currentColor" fill-opacity="0.5" />
    <circle cx="52" cy="70" r="1.2" fill="currentColor" fill-opacity="0.4" />
    <circle cx="32" cy="82" r="1.4" fill="currentColor" fill-opacity="0.35" />
  </g>
  <g class="text-bean-700">
    <rect x="20" y="34" width="76" height="3" fill="currentColor" />
    <rect x="55" y="37" width="3" height="58" fill="currentColor" />
    <rect x="16" y="96" width="84" height="4" rx="1" fill="currentColor" />
  </g>
  <!-- A pot of something growing on the sill. -->
  <g class="text-paprika-500">
    <path d="M62 96c0-7 3-11 8-11s8 4 8 11z" fill="currentColor" fill-opacity="0.8" />
  </g>
  <g class="text-herb-500">
    <path d="M70 85c-6-2-9-7-8-12 4 1 8 5 8 12zM70 85c6-3 9-8 8-13-5 1-8 6-8 13z" fill="currentColor" fill-opacity="0.75" />
  </g>

  <!-- The lamp, and the light it throws. -->
  <g class="text-honey-500">
    {#each GLOW as pool, i (i)}
      <ellipse cx="200" cy="150" rx={pool.rx} ry={pool.ry} fill="currentColor" fill-opacity={pool.o} />
    {/each}
  </g>
  <g class="text-espresso-900">
    <rect x="198" y="0" width="4" height="32" fill="currentColor" />
    <path d="M200 32c-22 0-34 16-38 30h76c-4-14-16-30-38-30z" fill="currentColor" />
  </g>
  <g class="text-honey-500">
    <ellipse cx="200" cy="63" rx="35" ry="7" fill="currentColor" fill-opacity="0.8" />
    <ellipse cx="200" cy="76" rx="21" ry="11" fill="currentColor" fill-opacity="0.3" />
  </g>

  <!-- The preserve shelves, at the edge of the light. -->
  <g class="text-bean-700">
    <rect x="292" y="104" width="108" height="5" fill="currentColor" />
    <rect x="292" y="152" width="108" height="5" fill="currentColor" />
  </g>
  {#each JARS as [jx, jw, jh, body, lid] (jx)}
    <g class={body}>
      <rect x={jx} y={104 - jh} width={jw} height={jh} rx="2.5" fill="currentColor" fill-opacity="0.85" />
    </g>
    <g class={lid}>
      <rect x={jx - 1} y={104 - jh - 3} width={jw + 2} height="4" rx="1.5" fill="currentColor" fill-opacity="0.9" />
    </g>
  {/each}
  {#each JARS_LOW as [jx, jw, jh, body, lid] (jx)}
    <g class={body}>
      <rect x={jx} y={152 - jh} width={jw} height={jh} rx="2.5" fill="currentColor" fill-opacity="0.85" />
    </g>
    <g class={lid}>
      <rect x={jx - 1} y={152 - jh - 3} width={jw + 2} height="4" rx="1.5" fill="currentColor" fill-opacity="0.9" />
    </g>
  {/each}

  <!-- A rail of utensils, silhouetted against the glow. -->
  <g class="text-espresso-900">
    <rect x="16" y="122" width="84" height="3" fill="currentColor" />
    <rect x="32" y="125" width="2.5" height="18" fill="currentColor" />
    <rect x="58" y="125" width="2.5" height="16" fill="currentColor" />
    <rect x="84" y="125" width="2.5" height="15" fill="currentColor" />
  </g>
  <g class="text-stone-500">
    <circle cx="33" cy="146" r="6" fill="currentColor" fill-opacity="0.7" />
    <ellipse cx="59" cy="146" rx="5" ry="7" fill="currentColor" fill-opacity="0.7" />
    <rect x="79" y="140" width="12" height="13" rx="3" fill="currentColor" fill-opacity="0.7" />
  </g>

  <!-- The two of them. The lit shape sits a little proud of the dark one, so their
       lamp-facing edge reads as a rim of light. -->
  {#each MICE as m (m.x)}
    <g class="text-caramel-400">
      <circle cx={m.x - 19 + m.dir * 4} cy={m.y - 28} r="14" fill="currentColor" />
      <circle cx={m.x + 19 + m.dir * 4} cy={m.y - 28} r="14" fill="currentColor" />
      <circle cx={m.x + m.dir * 4} cy={m.y} r="27" fill="currentColor" />
      <path d="M{m.x - 40 + m.dir * 4} 206c-3-38 12-62 40-62s43 24 40 62z" fill="currentColor" />
      <ellipse cx={m.x + m.dir * 26} cy={m.y + 10} rx="14" ry="10" fill="currentColor" />
    </g>
    <g class="text-bean-700">
      <circle cx={m.x - 19} cy={m.y - 28} r="14" fill="currentColor" />
      <circle cx={m.x + 19} cy={m.y - 28} r="14" fill="currentColor" />
      <circle cx={m.x} cy={m.y} r="27" fill="currentColor" />
      <path d="M{m.x - 40} 206c-3-38 12-62 40-62s43 24 40 62z" fill="currentColor" />
      <ellipse cx={m.x + m.dir * 22} cy={m.y + 10} rx="14" ry="10" fill="currentColor" />
    </g>
    <!-- Ear interiors, a nose and a lit eye catch the warmth. -->
    <g class="text-plum-500">
      <circle cx={m.x - 19} cy={m.y - 28} r="7" fill="currentColor" fill-opacity="0.6" />
      <circle cx={m.x + 19} cy={m.y - 28} r="7" fill="currentColor" fill-opacity="0.6" />
      <ellipse cx={m.x + m.dir * 33} cy={m.y + 7} rx="4" ry="3.2" fill="currentColor" fill-opacity="0.8" />
    </g>
    <g class="text-oat-200">
      <circle cx={m.x + m.dir * 10} cy={m.y - 6} r="3.4" fill="currentColor" fill-opacity="0.9" />
    </g>
    <!-- An apron, catching the lamp. -->
    <g class="text-cream-200">
      <path d="M{m.x - 22} 206c-1-26 6-38 22-38s23 12 22 38z" fill="currentColor" fill-opacity="0.22" />
    </g>
  {/each}

  <!-- Steam, drifting up into the light. -->
  <g class="text-cream-100">
    <path d="M196 178c-8-12 4-18-2-28 10 8 2 18 8 28z" fill="currentColor" fill-opacity="0.22" />
    <path d="M211 176c-7-10 4-16-2-24 9 7 2 16 7 24z" fill="currentColor" fill-opacity="0.15" />
  </g>

  <!-- The counter, catching the lamp along its edge. -->
  <g class="text-coffee-600">
    <rect x="0" y="206" width="400" height="26" fill="currentColor" />
  </g>
  <g class="text-caramel-400">
    <rect x="0" y="206" width="400" height="5" fill="currentColor" fill-opacity="0.85" />
  </g>

  <!-- The pot, still on. -->
  <g class="text-stone-700">
    <rect x="176" y="182" width="48" height="24" rx="5" fill="currentColor" />
    <rect x="170" y="177" width="60" height="6" rx="3" fill="currentColor" />
  </g>
  <g class="text-pesto-500">
    <rect x="180" y="184" width="40" height="5" rx="2" fill="currentColor" fill-opacity="0.7" />
  </g>

  <!-- The board, abandoned half-chopped: chillies and herbs still on it. -->
  <g class="text-cocoa-500">
    <rect x="36" y="192" width="56" height="14" rx="3" fill="currentColor" />
  </g>
  <g class="text-chilli-500">
    <ellipse cx="50" cy="198" rx="6" ry="4" fill="currentColor" fill-opacity="0.9" />
    <ellipse cx="62" cy="199" rx="5" ry="3.5" fill="currentColor" fill-opacity="0.9" />
  </g>
  <g class="text-herb-500">
    <ellipse cx="76" cy="198" rx="7" ry="4" fill="currentColor" fill-opacity="0.85" />
  </g>

  <!-- A bowl of fruit at the near end. -->
  <g class="text-stone-400">
    <path d="M292 206c-3-13 6-20 17-20s20 7 17 20z" fill="currentColor" fill-opacity="0.75" />
  </g>
  <g class="text-orchard-500">
    <circle cx="303" cy="189" r="5.5" fill="currentColor" fill-opacity="0.95" />
  </g>
  <g class="text-citrus-500">
    <circle cx="314" cy="188" r="5" fill="currentColor" fill-opacity="0.95" />
  </g>
  <g class="text-berry-500">
    <circle cx="309" cy="182" r="3.5" fill="currentColor" fill-opacity="0.95" />
  </g>

  <!-- A bottle, and two glasses poured. -->
  <g class="text-beet-500">
    <path d="M108 206v-26c0-6 4-7 4-11v-6h8v6c0 4 4 5 4 11v26z" fill="currentColor" fill-opacity="0.85" />
  </g>
  <g class="text-cream-100">
    <path d="M136 186h17l-3 20h-11z" fill="currentColor" fill-opacity="0.14" />
    <path d="M254 184h17l-3 22h-11z" fill="currentColor" fill-opacity="0.14" />
  </g>
  <g class="text-plum-500">
    <path d="M138 193h13l-2.5 13h-8z" fill="currentColor" fill-opacity="0.9" />
    <path d="M256 192h13l-2.5 14h-8z" fill="currentColor" fill-opacity="0.9" />
  </g>
  <g class="text-oat-200">
    <path d="M136 186h17v2h-17zM254 184h17v2h-17z" fill="currentColor" fill-opacity="0.45" />
    <path d="M139 196h3v8h-3zM257 195h3v9h-3z" fill="currentColor" fill-opacity="0.16" />
  </g>

  <!-- Salt, pepper, and a loaf someone brought. -->
  <g class="text-cream-300">
    <rect x="340" y="192" width="9" height="14" rx="2.5" fill="currentColor" fill-opacity="0.8" />
  </g>
  <g class="text-spice-500">
    <rect x="353" y="194" width="9" height="12" rx="2.5" fill="currentColor" fill-opacity="0.85" />
  </g>
  <g class="text-latte-300">
    <path d="M368 206c-2-11 5-17 14-17s16 6 14 17z" fill="currentColor" fill-opacity="0.9" />
  </g>

  <!-- The cabinets, the darkest thing in the room. -->
  <g class="text-espresso-900">
    <rect x="0" y="232" width="400" height="68" fill="currentColor" />
  </g>
  <g class="text-bean-700">
    <rect x="0" y="232" width="400" height="2" fill="currentColor" />
    <rect x="98" y="238" width="2" height="62" fill="currentColor" />
    <rect x="200" y="238" width="2" height="62" fill="currentColor" />
    <rect x="302" y="238" width="2" height="62" fill="currentColor" />
  </g>
  <g class="text-stone-600">
    <rect x="38" y="252" width="24" height="2.5" rx="1" fill="currentColor" fill-opacity="0.8" />
    <rect x="140" y="252" width="24" height="2.5" rx="1" fill="currentColor" fill-opacity="0.8" />
    <rect x="242" y="252" width="24" height="2.5" rx="1" fill="currentColor" fill-opacity="0.8" />
    <rect x="344" y="252" width="24" height="2.5" rx="1" fill="currentColor" fill-opacity="0.8" />
  </g>
  <!-- A tea towel over one of the handles. -->
  <g class="text-allium-500">
    <path d="M242 254h14v28c0 3-14 3-14 0z" fill="currentColor" fill-opacity="0.55" />
  </g>
</svg>
