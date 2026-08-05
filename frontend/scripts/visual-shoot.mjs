#!/usr/bin/env node
/**
 * Full-page, deterministic story screenshots for visual regression.
 *
 * The `storybook-shot` flake task captures a fixed viewport, which crops a long
 * page — and a cropped page means a change below the fold lands unreviewed, which
 * is the one thing this fence exists to prevent. So this uses puppeteer's
 * `fullPage`: every story is captured whole, however tall.
 *
 * Determinism is the whole game (a baseline you cannot reproduce is noise):
 *   - the pinned nix chromium (CHROMIUM_BIN), same as CI
 *   - self-hosted fonts + FONTCONFIG_FILE, so no network and no fallback
 *   - a fixed width + deviceScaleFactor, animations disabled, and a wait on
 *     document.fonts.ready so text is never captured mid-swap, and every
 *     image's decode() so a photograph is never captured mid-decode.
 *   - every image the app itself ships renders unresampled — one source pixel per
 *     device pixel — asserted on every shot. This is the one thing above that no
 *     amount of waiting can fix: see `assertUnresampled` (#223).
 *   - external images (fixtures point at real themealdb.com photos) are stubbed
 *     with a fixed local placeholder. The regression fence protects layout, not a
 *     remote CDN's pixels — an unstubbed photo would make the shot depend on the
 *     network and go red whenever a source rotates an image. The manual
 *     `storybook-shot` PR flow keeps the real photos; this one must not.
 *
 *   CHROMIUM_BIN=… FONTCONFIG_FILE=… node scripts/visual-shoot.mjs
 */
import { createServer } from "node:http";
import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { extname, join } from "node:path";
import puppeteer from "puppeteer-core";
import { PNG } from "pngjs";

const ROOT = new URL("..", import.meta.url).pathname;
const SB_DIR = join(ROOT, "storybook-static");
const OUT = join(ROOT, "tests/visual/current");
const WIDTH = Number(process.env.WIDTH || 900);
const CHROMIUM = process.env.CHROMIUM_BIN;

// A fixed placeholder every external image resolves to, so a card's image-present
// layout renders identically without touching the network. Stone-toned to sit in
// the cream room; `object-fit` scales it to fill, so the size is irrelevant.
const PLACEHOLDER = (() => {
  const png = new PNG({ width: 8, height: 8 });
  for (let i = 0; i < png.data.length; i += 4) {
    png.data[i] = 201;
    png.data[i + 1] = 193;
    png.data[i + 2] = 181;
    png.data[i + 3] = 255;
  }
  return PNG.sync.write(png);
})();

if (!CHROMIUM) {
  console.error(
    "visual-shoot: CHROMIUM_BIN is required (the pinned nix chromium)",
  );
  process.exit(1);
}

/**
 * Runs in the page: every image served from our own origin must land on the device
 * pixel grid exactly — one source pixel per device pixel, nothing to resample.
 *
 * A photograph drawn at a fractional scale has a partly covered destination column at
 * its edge, and the browser has more than one way to resolve it. Which one it takes is
 * a raster-path decision, and under load it does not always take the same one, so two
 * shoots of the *same build* differ in that column — the fence reporting a change the
 * app did not make, which defeats it (#223). At a scale of exactly 1 there is nothing
 * to resolve: the destination is the source, whatever filter the rasterizer picks.
 *
 * The backdrops ship as a `srcset` ladder, so this works from `currentSrc` — the
 * variant the browser actually *chose*, not the one the markup lists first. That makes
 * it a check on the choice as well as on the file: the fence pins the viewport and the
 * device scale, so the selection is a pure function of two fixed numbers, and a
 * selection that landed on another rung would stop being 1:1 and be named here.
 *
 * It cannot use `naturalWidth` of the rendered element to do that. On an image with a
 * `srcset`, that property is *density-corrected* — the browser divides the file's real
 * pixels by the density of the candidate it picked and reports CSS pixels, so a 1800px
 * file chosen at 2x reports 900 and every ladder would look like it was drawn at 2x.
 * The chosen file is re-loaded on its own, without a `srcset` to correct against, which
 * is the only way to ask what it actually is.
 *
 * Only our own assets are held to this. The external fixtures are answered with a flat
 * 8x8 placeholder below, and no scale can change a pixel of one flat colour.
 *
 * Returns a list of offenders (empty when everything is on the grid).
 */
const assertUnresampled = async (origin) => {
  const off = [];
  for (const img of document.images) {
    if (!img.currentSrc.startsWith(origin)) continue;
    const probe = new Image();
    probe.src = img.currentSrc;
    await probe.decode().catch(() => {});
    const sourceW = probe.naturalWidth;
    const sourceH = probe.naturalHeight;
    if (!sourceW || !sourceH) continue;
    const box = img.getBoundingClientRect();
    const fit = getComputedStyle(img).objectFit;
    const x = box.width / sourceW;
    const y = box.height / sourceH;
    // `fill` stretches the axes independently, so it has no single scale — it can only
    // be unresampled when both axes are, which is what comparing them each to 1 does.
    const scales = fit === "cover"
      ? [Math.max(x, y)]
      : fit === "contain" || fit === "scale-down"
      ? [Math.min(x, y)]
      : fit === "none"
      ? [1]
      : [x, y];
    if (scales.every((s) => s * devicePixelRatio === 1)) continue;
    off.push(
      `${
        new URL(img.currentSrc).pathname
      } is ${sourceW}x${sourceH} drawn into ${box.width}x${box.height} (object-fit: ${fit}) at ${
        scales.map((s) => (s * devicePixelRatio).toFixed(4)).join("x")
      } device px per source px`,
    );
  }
  return off;
};

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".json": "application/json",
  ".css": "text/css",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

// A tiny static server: iframe.html loads ES modules, so file:// will not do.
const server = createServer((req, res) => {
  try {
    const path = decodeURIComponent(req.url.split("?")[0]);
    const file = join(SB_DIR, path === "/" ? "index.html" : path);
    const body = readFileSync(file);
    res.writeHead(200, {
      "content-type": MIME[extname(file)] || "application/octet-stream",
    });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end();
  }
});

const index = JSON.parse(readFileSync(join(SB_DIR, "index.json"), "utf8"));
const ids = Object.entries(index.entries)
  .filter(([, e]) => e.type === "story")
  .map(([id]) => id)
  .sort();

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;

const browser = await puppeteer.launch({
  executablePath: CHROMIUM,
  headless: true,
  args: [
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--hide-scrollbars",
    "--force-color-profile=srgb",
  ],
});

try {
  const page = await browser.newPage();
  await page.setViewport({ width: WIDTH, height: 800, deviceScaleFactor: 2 });
  // Everything the story legitimately needs is served from 127.0.0.1 (the bundle,
  // the self-hosted fonts). Any other request is an external image fixture —
  // answer images with the fixed placeholder, and refuse anything else, so the
  // render is fully offline and can't drift with a remote host.
  await page.setRequestInterception(true);
  page.on("request", (req) => {
    if (req.url().includes(`127.0.0.1:${port}`)) return void req.continue();
    if (req.resourceType() === "image") {
      return void req.respond({
        status: 200,
        contentType: "image/png",
        body: PLACEHOLDER,
      });
    }
    return void req.abort();
  });
  // Kill animations/transitions so a shot is never caught mid-motion.
  await page.evaluateOnNewDocument(() => {
    const css =
      "*,*::before,*::after{transition:none!important;animation:none!important;caret-color:transparent!important}";
    const style = document.createElement("style");
    style.textContent = css;
    document.documentElement.appendChild(style);
  });

  for (const id of ids) {
    await page.goto(
      `http://127.0.0.1:${port}/iframe.html?id=${id}&viewMode=story`,
      {
        waitUntil: "networkidle0",
        timeout: 30000,
      },
    );
    await page.evaluate(async () => {
      await document.fonts.ready;
      // The same wait, for images. `networkidle0` only says the bytes arrived —
      // decoding them into a paintable bitmap is a separate asynchronous step, so a
      // large photograph can still be resolving when the shot is taken. That does not
      // look like a missing image; it looks like a faint difference spread across the
      // whole picture, on one run and not the next. A broken or stubbed image rejects
      // here, which must not stall the shoot.
      await Promise.all(
        [...document.images].map((img) => img.decode().catch(() => {})),
      );
    });
    const resampled = await page.evaluate(
      assertUnresampled,
      `http://127.0.0.1:${port}/`,
    );
    if (resampled.length) {
      console.error(
        `visual-shoot: ${id} draws an app image at a scale other than 1:1, so its\n` +
          `pixels are a raster-path decision and two shoots of this build can differ:\n` +
          resampled.map((r) => `  ${r}\n`).join("") +
          `Re-export the asset at the size it is drawn — do not widen the diff budget.`,
      );
      process.exitCode = 1;
      break;
    }
    await page.screenshot({ path: join(OUT, `${id}.png`), fullPage: true });
    console.log(id);
  }
} finally {
  await browser.close();
  server.close();
}
