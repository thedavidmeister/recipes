<script lang="ts">
  import Button from "./Button.svelte";
  /**
   * The design system, as a page you can look at. Not shipped in the app — it is
   * the reference we screenshot and argue about.
   *
   * The vibe: fun and playful but utilitarian; warm, cozy, delicious. The rule
   * holding it together is **brown is the room, bright is the food** — surfaces
   * and words are warm neutrals, and colour is spent only on what you act on and
   * what you made.
   */

  const creams = [
    { name: "cream-50", var: "--color-cream-50", note: "paper" },
    { name: "cream-100", var: "--color-cream-100", note: "raised" },
    { name: "cream-200", var: "--color-cream-200", note: "sunken" },
    { name: "cream-300", var: "--color-cream-300", note: "edge" },
  ];

  const stones = [
    { name: "stone-200", var: "--color-stone-200" },
    { name: "stone-300", var: "--color-stone-300" },
    { name: "stone-400", var: "--color-stone-400" },
    { name: "stone-500", var: "--color-stone-500" },
    { name: "stone-600", var: "--color-stone-600" },
    { name: "stone-700", var: "--color-stone-700" },
    { name: "stone-800", var: "--color-stone-800" },
    { name: "stone-900", var: "--color-stone-900" },
  ];

  const browns = [
    { name: "oat-200", var: "--color-oat-200" },
    { name: "latte-300", var: "--color-latte-300" },
    { name: "caramel-400", var: "--color-caramel-400" },
    { name: "cocoa-500", var: "--color-cocoa-500" },
    { name: "coffee-600", var: "--color-coffee-600" },
    { name: "bean-700", var: "--color-bean-700" },
    { name: "espresso-800", var: "--color-espresso-800" },
    { name: "espresso-900", var: "--color-espresso-900" },
  ];

  const foods = [
    {
      name: "pesto",
      tone: "--color-pesto-500",
      tint: "--color-pesto-100",
      role: "action · pick",
    },
    {
      name: "plum",
      tone: "--color-plum-500",
      tint: "--color-plum-100",
      role: "ripe · buy",
    },
    {
      name: "paprika",
      tone: "--color-paprika-500",
      tint: "--color-paprika-100",
      role: "heat · cook",
    },
    {
      name: "honey",
      tone: "--color-honey-500",
      tint: "--color-honey-100",
      role: "sweet · joy",
    },
  ];

  const weights = [
    { w: 400, label: "Regular" },
    { w: 600, label: "Semibold" },
    { w: 800, label: "Extrabold" },
  ];

  // Vertical rhythm: named gaps, each a whole number of 8px beats.
  const rhythm = [
    {
      name: "tight",
      px: 8,
      beats: "1 beat",
      use: "a heading & its own line; within a component",
    },
    { name: "block", px: 16, beats: "2 beats", use: "paragraph to paragraph" },
    {
      name: "group",
      px: 24,
      beats: "3 beats",
      use: "a label to its content; card padding",
    },
    { name: "sub", px: 32, beats: "4 beats", use: "between subsections" },
    {
      name: "section",
      px: 56,
      beats: "7 beats",
      use: "between major sections",
    },
  ];

  // Colour-code ingredients by flavour family (#40's taxonomy). Curated to
  // evoke, not spread on a wheel. roasted/nutty = the browns; dairy = the creams.
  const flavours = [
    {
      name: "chilli",
      base: "--color-chilli-500",
      tint: "--color-chilli-100",
      eg: "chilli · pepper · ginger",
    },
    {
      name: "citrus",
      base: "--color-citrus-500",
      tint: "--color-citrus-100",
      eg: "lemon · lime · orange",
    },
    {
      name: "orchard",
      base: "--color-orchard-500",
      tint: "--color-orchard-100",
      eg: "apple · peach · pear",
    },
    {
      name: "spice",
      base: "--color-spice-500",
      tint: "--color-spice-100",
      eg: "cumin · paprika · curry",
    },
    {
      name: "allium",
      base: "--color-allium-500",
      tint: "--color-allium-100",
      eg: "garlic · onion · leek",
    },
    {
      name: "herb",
      base: "--color-herb-500",
      tint: "--color-herb-100",
      eg: "basil · mint · parsley",
    },
    {
      name: "sea",
      base: "--color-sea-500",
      tint: "--color-sea-100",
      eg: "fish · seaweed · samphire",
    },
    {
      name: "berry",
      base: "--color-berry-500",
      tint: "--color-berry-100",
      eg: "blackberry · currant · grape",
    },
    {
      name: "floral",
      base: "--color-floral-500",
      tint: "--color-floral-100",
      eg: "rose · saffron · vanilla",
    },
    {
      name: "beet",
      base: "--color-beet-500",
      tint: "--color-beet-100",
      eg: "beetroot · rhubarb · pomegranate",
    },
    {
      name: "roasted",
      base: "--color-cocoa-500",
      tint: "--color-cream-200",
      eg: "coffee · caramel · walnut",
    },
    {
      name: "dairy",
      base: "--color-latte-300",
      tint: "--color-cream-100",
      eg: "milk · cheese · butter",
    },
  ];
</script>

<div class="bg-cream-50 min-h-screen px-6 py-12 text-stone-900 sm:px-10">
  <div class="mx-auto max-w-3xl">
    <!-- Masthead -->
    <header class="pb-10">
      <p
        class="text-sm font-semibold tracking-[0.15em] text-stone-400 uppercase"
      >
        Design system
      </p>
      <h1
        class="font-display mt-4 text-5xl leading-tight font-medium tracking-tight text-stone-900"
      >
        warm, cozy, delicious
      </h1>
      <p class="mt-4 max-w-xl text-lg text-stone-600">
        A game, not a cookbook. Cream paper, warm-grey ink, bright food. Calm
        and confident — the play is the fun, not the chrome.
      </p>
    </header>

    <!-- Colour: the room -->
    <section class="mt-14">
      <h2 class="font-display text-3xl font-medium">the room</h2>
      <p class="mt-1 text-stone-600">
        Cream paper, warm-grey ink. Brown is an accent, not the default.
      </p>

      <p
        class="mt-6 text-sm font-semibold tracking-wider text-stone-500 uppercase"
      >
        Creams — paper, never white
      </p>
      <div class="mt-3 grid grid-cols-2 gap-3 sm:grid-cols-4">
        {#each creams as c (c.name)}
          <div class="overflow-hidden rounded-2xl border border-stone-200">
            <div class="h-20" style="background: var({c.var})"></div>
            <div class="bg-cream-100 px-3 py-2">
              <p class="font-semibold">{c.name}</p>
              <p class="text-sm text-stone-500">{c.note}</p>
            </div>
          </div>
        {/each}
      </div>

      <p
        class="mt-8 text-sm font-semibold tracking-wider text-stone-500 uppercase"
      >
        Warm grey — the ink &amp; the borders
      </p>
      <div
        class="mt-3 flex overflow-hidden rounded-2xl border border-stone-200"
      >
        {#each stones as st (st.name)}
          <div class="flex-1">
            <div class="h-24" style="background: var({st.var})"></div>
            <div
              class="bg-cream-100 px-1 py-2 text-center text-[11px] font-medium text-stone-500"
            >
              {st.name.split("-")[1]}
            </div>
          </div>
        {/each}
      </div>

      <p
        class="mt-8 text-sm font-semibold tracking-wider text-stone-500 uppercase"
      >
        Brown — an accent, not the neutral (also the roasted flavour)
      </p>
      <div
        class="mt-3 flex overflow-hidden rounded-2xl border border-stone-200"
      >
        {#each browns as b (b.name)}
          <div class="flex-1">
            <div class="h-16" style="background: var({b.var})"></div>
          </div>
        {/each}
      </div>
    </section>

    <!-- Colour: the food -->
    <section class="mt-14">
      <h2 class="font-display text-3xl font-medium">the food</h2>
      <p class="mt-1 text-stone-600">
        Bright secondaries, named after what you'd eat. One per section, used as
        a spice.
      </p>
      <div class="mt-6 grid grid-cols-2 gap-4 sm:grid-cols-4">
        {#each foods as f (f.name)}
          <div
            class="bg-cream-100 overflow-hidden rounded-3xl border border-stone-200"
          >
            <div class="flex h-24" style="background: var({f.tint})">
              <div
                class="m-auto size-12 rounded-full"
                style="background: var({f.tone})"
              ></div>
            </div>
            <div class="px-4 py-3">
              <p class="font-display text-xl font-semibold capitalize">
                {f.name}
              </p>
              <p class="text-sm text-stone-500">{f.role}</p>
            </div>
          </div>
        {/each}
      </div>
    </section>

    <!-- Flavours -->
    <section class="mt-14">
      <h2 class="font-display text-3xl font-medium">the flavours</h2>
      <p class="mt-1 text-stone-600">
        Ingredients coloured by what they taste like (#40). Curated to evoke,
        not spread on a wheel — the chip is the flavour, the text stays
        espresso.
      </p>
      <div class="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-2">
        {#each flavours as f (f.name)}
          <div
            class="flex items-center gap-3 rounded-2xl px-4 py-3"
            style="background: var({f.tint})"
          >
            <span
              class="size-5 shrink-0 rounded-full"
              style="background: var({f.base})"
            ></span>
            <div class="min-w-0">
              <p
                class="font-display text-lg leading-tight font-semibold text-stone-900 capitalize"
              >
                {f.name}
              </p>
              <p class="truncate text-sm text-stone-500">{f.eg}</p>
            </div>
          </div>
        {/each}
      </div>
      <p class="mt-4 text-sm text-stone-500">
        Roasted, nutty and earthy flavours <em>are</em> the browns; dairy is the
        creams — so the room doubles as two more flavour families.
      </p>
    </section>

    <!-- Type -->
    <section class="mt-14">
      <h2 class="font-display text-3xl font-medium">the type</h2>
      <p class="mt-1 text-stone-600">
        Fredoka for the fun, Nunito for the work. No serif — a serif is a
        cookbook, this is a game.
      </p>

      <div
        class="bg-cream-100 mt-6 rounded-3xl border border-stone-200 p-6 sm:p-8"
      >
        <p class="text-sm font-bold tracking-wider text-stone-500 uppercase">
          Rubik — display, rounded &amp; assured
        </p>
        <p
          class="font-display mt-3 text-5xl leading-tight font-medium text-stone-900"
        >
          Sticky sesame noodles
        </p>
        <p class="font-display mt-2 text-2xl font-normal text-stone-600">
          rounded and friendly, said quietly
        </p>
      </div>

      <div
        class="bg-cream-100 mt-4 rounded-3xl border border-stone-200 p-6 sm:p-8"
      >
        <p class="text-sm font-bold tracking-wider text-stone-500 uppercase">
          Nunito — body, rounded &amp; readable
        </p>
        <div class="mt-3 space-y-1">
          {#each weights as { w, label } (w)}
            <p class="text-xl" style="font-weight: {w}">
              <span class="text-stone-500">{label}</span> — toast the sesame until
              it smells like a warning, then pull it off the heat.
            </p>
          {/each}
        </div>
      </div>
    </section>

    <!-- Vertical rhythm -->
    <section class="mt-14">
      <h2 class="font-display text-3xl font-medium">the rhythm</h2>
      <p class="mt-1 text-stone-600">
        Vertical space is a system, not a guess. An 8px beat; body sits on a
        24px line (three beats), and every gap is a whole number of beats.
      </p>

      <!-- The beat scale -->
      <div class="mt-6 space-y-2">
        {#each rhythm as r (r.name)}
          <div class="flex items-center gap-4">
            <div class="w-24 shrink-0 text-right">
              <p class="font-semibold text-stone-900">{r.name}</p>
              <p class="text-xs text-stone-500">{r.px}px · {r.beats}</p>
            </div>
            <div
              class="bg-pesto-500 h-3 rounded-full"
              style="width: {r.px}px"
            ></div>
            <p class="text-sm text-stone-500">{r.use}</p>
          </div>
        {/each}
      </div>

      <!-- Baseline demo: text sitting on the 24px grid -->
      <p
        class="mt-8 text-sm font-semibold tracking-wider text-stone-500 uppercase"
      >
        On the grid — every line &amp; gap lands on a 24px beat
      </p>
      <div
        class="mt-3 overflow-hidden rounded-3xl border border-stone-200 p-6 sm:p-8"
        style="background-image: repeating-linear-gradient(to bottom, var(--color-cream-100) 0, var(--color-cream-100) 23px, var(--color-stone-200) 23px, var(--color-stone-200) 24px);"
      >
        <h3
          class="font-display font-medium text-stone-900"
          style="font-size: 24px; line-height: 24px; margin: 0;"
        >
          Sticky sesame noodles
        </h3>
        <p
          class="text-stone-600"
          style="font-size: 16px; line-height: 24px; margin-top: 8px;"
        >
          The heading and its line are one unit, eight pixels apart. This
          paragraph sits on the grid at sixteen-pixel leading rounded to the
          beat, so each line lands on a rule.
        </p>
        <p
          class="text-stone-600"
          style="font-size: 16px; line-height: 24px; margin-top: 16px;"
        >
          A second paragraph, two beats down — the gap between blocks is bigger
          than the gap inside one, and both are whole beats, so nothing drifts.
        </p>
      </div>

      <p class="mt-4 text-sm text-stone-500">
        heading → its line <span class="text-stone-700">8</span> · paragraph →
        paragraph <span class="text-stone-700">16</span> · label → content
        <span class="text-stone-700">24</span> · section → section
        <span class="text-stone-700">56</span>. Half-beats (4px) exist for
        icon-to-label only.
      </p>
    </section>

    <!-- Components -->
    <section class="mt-14">
      <h2 class="font-display text-3xl font-medium">the pieces</h2>
      <p class="mt-1 text-stone-600">
        Big, round, generous. Colour only where you act.
      </p>

      <!-- Buttons -->
      <div class="mt-6 flex flex-wrap items-center gap-3">
        <!-- The real component, not a copy of it: a swatch that restates the markup
             is a swatch that goes quietly out of date. -->
        <Button dot="pesto">Cook this</Button>
        <Button variant="secondary">Maybe later</Button>
        <Button variant="quiet">skip</Button>
      </div>

      <!-- A card -->
      <div
        class="bg-cream-100 mt-6 overflow-hidden rounded-[1.75rem] border border-stone-200"
      >
        <div class="flex h-44 items-center justify-center bg-stone-100">
          <span class="font-display text-lg font-medium text-stone-400"
            >photo</span
          >
        </div>
        <div class="p-6">
          <div class="flex flex-wrap items-center gap-2">
            <span
              class="bg-herb-100 text-herb-500 rounded-full px-3 py-1 text-sm font-semibold"
            >
              15 min
            </span>
            <span
              class="bg-chilli-100 text-chilli-500 rounded-full px-3 py-1 text-sm font-semibold"
            >
              spicy
            </span>
            <span
              class="bg-allium-100 text-allium-500 rounded-full px-3 py-1 text-sm font-semibold"
            >
              garlicky
            </span>
          </div>
          <h3
            class="font-display mt-3 text-2xl leading-tight font-medium text-stone-900"
          >
            Miso butter greens
          </h3>
          <p class="mt-1 text-stone-600">
            The kind of thing you make on a Tuesday and feel unreasonably
            pleased about.
          </p>
        </div>
      </div>

      <!-- Pills / tags -->
      <div class="mt-6 flex flex-wrap gap-2">
        {#each foods as f (f.name)}
          <span
            class="rounded-full px-4 py-1.5 font-bold capitalize"
            style="background: var({f.tint}); color: var({f.tone})"
          >
            {f.name}
          </span>
        {/each}
      </div>
    </section>
  </div>
</div>
