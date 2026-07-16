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
 *     document.fonts.ready so text is never captured mid-swap.
 *
 *   CHROMIUM_BIN=… FONTCONFIG_FILE=… node scripts/visual-shoot.mjs
 */
import { createServer } from "node:http";
import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { extname, join } from "node:path";
import puppeteer from "puppeteer-core";

const ROOT = new URL("..", import.meta.url).pathname;
const SB_DIR = join(ROOT, "storybook-static");
const OUT = join(ROOT, "tests/visual/current");
const WIDTH = Number(process.env.WIDTH || 900);
const CHROMIUM = process.env.CHROMIUM_BIN;

if (!CHROMIUM) {
  console.error(
    "visual-shoot: CHROMIUM_BIN is required (the pinned nix chromium)",
  );
  process.exit(1);
}

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
    });
    await page.screenshot({ path: join(OUT, `${id}.png`), fullPage: true });
    console.log(id);
  }
} finally {
  await browser.close();
  server.close();
}
