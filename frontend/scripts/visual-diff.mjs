#!/usr/bin/env node
/**
 * Visual regression — the fence the token lint can't build.
 *
 * The design lint reads source: it proves a surface *uses* the tokens, never
 * that it still *looks* right. A heading colliding with a button, a card
 * overflowing, a font silently falling back (exactly the Fraunces-wght bug) — all
 * pass the source lint and wreck the render. So: every story has a committed
 * baseline PNG; CI re-renders and pixel-diffs; any change fails until someone
 * looks at the diff and re-blesses the baseline.
 *
 * "Someone" is the point: a failure is not a wall, it is *feedback to read* —
 * the same way you read failing-test output. So a changed story does not just
 * emit a magenta diff (which shows *where* pixels moved, not whether the new
 * look is right). It emits a `baseline | current | diff` triptych: the before,
 * the after, and the delta, side by side in one PNG. Open that one image and you
 * can judge the change — intended and good (re-bless), intended but wrong (fix),
 * or a regression you never meant (revert). CI uploads these on failure so the
 * reviewer — human or agent — reads the actual pixels, not a summary of them.
 *
 * This consumes shots the `visual-shoot` harness already produced (pinned
 * chromium + the fonts.conf make them deterministic — the hard part). It only
 * diffs.
 *
 *   node scripts/visual-diff.mjs            # compare current/ vs baselines/, fail on drift
 *   node scripts/visual-diff.mjs --update   # (re)write baselines from current/
 *
 * Determinism note: baselines are generated in the same nix environment CI runs
 * in (rainix pins chromium + freetype + fontconfig), so cross-machine subpixel
 * AA is not a moving target. A small threshold absorbs any residual noise; a real
 * change dwarfs it.
 */
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, join } from "node:path";
import { PNG } from "pngjs";
import pixelmatch from "pixelmatch";

const ROOT = new URL("..", import.meta.url).pathname;
const CURRENT = join(ROOT, "tests/visual/current");
const BASELINES = join(ROOT, "tests/visual/baselines");
const DIFFS = join(ROOT, "tests/visual/__diff__");

// pixelmatch per-pixel sensitivity (0 strict … 1 loose). 0.1 ignores AA jitter.
const PER_PIXEL = 0.1;
// Absolute changed-pixel budget before a story is called changed. Measured: two
// independent runs in the pinned nix env diff by *exactly 0px*, so this is not a
// noise allowance — it is a hair of slack for a theoretical cross-machine AA
// fringe. A real restyle is thousands of pixels; even a colour tweak on a tiny
// element (a nav "you are here" ring) is hundreds, so nothing meaningful slips a
// budget this small. A ratio threshold was wrong here: it scales with page
// height, so a small-element change on a tall page hid under it.
const MAX_CHANGED = 8;

const update = process.argv.includes("--update");

const shots = existsSync(CURRENT)
  ? readdirSync(CURRENT).filter((f) => f.endsWith(".png"))
  : [];

if (shots.length === 0) {
  console.error(
    "visual: no current shots — build storybook and run the shot harness first",
  );
  process.exit(1);
}

if (update) {
  mkdirSync(BASELINES, { recursive: true });
  // Replace wholesale so a removed story doesn't leave a stale baseline behind.
  for (const f of readdirSync(BASELINES)) rmSync(join(BASELINES, f));
  for (const f of shots) copyFileSync(join(CURRENT, f), join(BASELINES, f));
  console.log(`visual: wrote ${shots.length} baseline(s).`);
  process.exit(0);
}

if (!existsSync(BASELINES) || readdirSync(BASELINES).length === 0) {
  console.error(
    "visual: no baselines — run `npm run visual:update` and commit tests/visual/baselines",
  );
  process.exit(1);
}

const baselines = new Set(
  readdirSync(BASELINES).filter((f) => f.endsWith(".png")),
);
const current = new Set(shots);

const failures = [];

// A magenta diff shows *where* pixels moved; it can't show whether the new look
// is right. So stitch the three images that answer that — baseline (before),
// current (after), diff (delta) — into one PNG, side by side, so a single glance
// judges the change. Panels are separated by a stone gutter and topped by a thin
// colour-coded bar (green = before, tomato = after, magenta = delta) so the order
// reads without a caption.
const GUTTER = 24;
const BAR = 12;
const BG = [245, 242, 236]; // cream-50-ish, matches the app room
const BARS = [
  [122, 165, 74],
  [239, 95, 60],
  [255, 0, 255],
];
function triptych(a, b, d) {
  const h = Math.max(a.height, b.height, d.height);
  const w = a.width + GUTTER + b.width + GUTTER + d.width;
  const out = new PNG({ width: w, height: h + BAR });
  for (let i = 0; i < out.data.length; i += 4) {
    out.data[i] = BG[0];
    out.data[i + 1] = BG[1];
    out.data[i + 2] = BG[2];
    out.data[i + 3] = 255;
  }
  let x = 0;
  [a, b, d].forEach((src, panel) => {
    const [br, bg, bb] = BARS[panel];
    for (let yy = 0; yy < BAR; yy++) {
      for (let xx = 0; xx < src.width; xx++) {
        const o = (yy * w + x + xx) * 4;
        out.data[o] = br;
        out.data[o + 1] = bg;
        out.data[o + 2] = bb;
        out.data[o + 3] = 255;
      }
    }
    for (let yy = 0; yy < src.height; yy++) {
      for (let xx = 0; xx < src.width; xx++) {
        const so = (yy * src.width + xx) * 4;
        const dOff = ((yy + BAR) * w + x + xx) * 4;
        out.data[dOff] = src.data[so];
        out.data[dOff + 1] = src.data[so + 1];
        out.data[dOff + 2] = src.data[so + 2];
        out.data[dOff + 3] = src.data[so + 3];
      }
    }
    x += src.width + GUTTER;
  });
  return out;
}

// A story with no baseline is unreviewed art; a baseline with no story is dead.
for (const f of current) {
  if (!baselines.has(f)) {
    failures.push({
      f,
      why: "new story with no baseline — run `npm run visual:update`",
    });
  }
}
for (const f of baselines) {
  if (!current.has(f)) {
    failures.push({
      f,
      why:
        "baseline for a story that no longer renders — run `npm run visual:update`",
    });
  }
}

rmSync(DIFFS, { recursive: true, force: true });
mkdirSync(DIFFS, { recursive: true });

for (const f of current) {
  if (!baselines.has(f)) continue;
  const a = PNG.sync.read(readFileSync(join(BASELINES, f)));
  const b = PNG.sync.read(readFileSync(join(CURRENT, f)));
  if (a.width !== b.width || a.height !== b.height) {
    failures.push({
      f,
      why: `size changed ${a.width}x${a.height} → ${b.width}x${b.height}`,
    });
    continue;
  }
  const diff = new PNG({ width: a.width, height: a.height });
  // pixelmatch renders unchanged pixels as a faded copy of the page and lights
  // changed ones — magenta here, to match the delta bar and pop off the warm room.
  const changed = pixelmatch(a.data, b.data, diff.data, a.width, a.height, {
    threshold: PER_PIXEL,
    diffColor: [255, 0, 255],
  });
  if (changed > MAX_CHANGED) {
    writeFileSync(join(DIFFS, f), PNG.sync.write(triptych(a, b, diff)));
    const ratio = changed / (a.width * a.height);
    failures.push({
      f,
      why: `${changed}px changed (${
        (ratio * 100).toFixed(
          3,
        )
      }%) — look: tests/visual/__diff__/${f}  (baseline | current | diff)`,
    });
  }
}

if (failures.length === 0) {
  console.log(`visual: clean — ${current.size} stories match their baselines.`);
  process.exit(0);
}

console.error(
  `visual: ${failures.length} change(s) — read each triptych like failing-test output before deciding:\n`,
);
for (const { f, why } of failures) {
  console.error(`  ${basename(f)}\n    → ${why}`);
}
console.error(
  `\n  intended and right → \`npm run visual:update\` re-blesses the baselines.`,
);
console.error(
  `  wrong or unexpected → it is a regression; fix the surface, do not re-bless.`,
);
process.exit(1);
