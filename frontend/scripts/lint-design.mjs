#!/usr/bin/env node
/**
 * Design-system enforcement — fail loud on the escape hatches.
 *
 * A design system nobody enforces decays into "what looked right that day". So
 * this is a CI gate, not a guideline: the build breaks if a surface reaches
 * outside the tokens. It bans the ways drift gets in, and every ban has a
 * one-line reason so the failure teaches rather than just blocks.
 *
 * It scans the SPA source only. `app.css` is exempt — that is where the tokens
 * are defined, so it is the one place raw values belong.
 *
 * No dependencies: a regex scan is enough, and a lint that needs its own build
 * step is a lint people disable.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const SRC = join(ROOT, "src");

// Files where raw values are legitimate: app.css defines the tokens.
const EXEMPT = [join(SRC, "app.css")];

// The Tailwind default colour families we do NOT define as tokens. Using one
// means reaching past the palette. `stone`, `cream`, `oat`/`latte`/… and the
// food/flavour names are ours (defined in @theme), so they are allowed.
const DEFAULT_FAMILIES = [
  "slate",
  "gray",
  "zinc",
  "neutral",
  "red",
  "orange",
  "amber",
  "yellow",
  "lime",
  "green",
  "emerald",
  "teal",
  "cyan",
  "sky",
  "blue",
  "indigo",
  "violet",
  "purple",
  "fuchsia",
  "pink",
  "rose",
];

const RULES = [
  {
    // A raw hex is a colour that skipped the palette entirely.
    re: /#[0-9a-fA-F]{3,8}\b/g,
    why:
      "raw hex — use a colour token (var(--color-…) or a Tailwind class from @theme)",
  },
  {
    // Tailwind's default palette, in any utility (bg-/text-/border-/ring-/from-…).
    re: new RegExp(
      String
        .raw`\b(?:bg|text|border|ring|from|via|to|fill|stroke|decoration|outline|divide|accent|caret|shadow)-(?:${
        DEFAULT_FAMILIES.join(
          "|",
        )
      })-\d{2,3}\b`,
      "g",
    ),
    why:
      "Tailwind default palette — use a design token (cream/stone/tomato/matcha/honey/plum/chilli/citrus/… )",
  },
  {
    re: /\b(?:bg|text|border|ring|fill|stroke)-white\b/g,
    why: "white — the room is cream, never white (bg-cream-50)",
  },
  {
    re: /\b(?:bg|text|border|ring|fill|stroke)-black\b/g,
    why: "black — ink is warm-grey (text-stone-900)",
  },
  {
    // Arbitrary colour: bg-[#..], text-[rgb(..)], etc. Geometry arbitraries are
    // fine; colour ones route around the palette.
    re: /-\[(?:#|rgb|hsl|oklch|oklab)[^\]]*\]/gi,
    why:
      "arbitrary colour value — add it to the palette instead of inlining it",
  },
  {
    re: /\bfont-serif\b/g,
    why:
      "serif — the display face is Rubik (font-display); a serif reads cookbook, not game",
  },
  {
    // An external font URL breaks two decisions at once: self-hosting (the
    // screenshot harness serves locally, so a CDN font renders as a fallback in
    // every shot) and not leaking to a third party. Fonts come from Fontsource.
    re:
      /fonts\.(?:googleapis|gstatic)\.com|@import\s+(?:url\()?["']?https?:\/\//gi,
    why:
      "external font/URL — fonts are self-hosted via Fontsource; a CDN font renders as a fallback in screenshots",
  },
  {
    // Off-beat spacing bypasses the rhythm. Margin/padding/gap must come from the
    // scale (multiples of the 8px beat), never an arbitrary pixel value. Position,
    // tracking, radius and font-size arbitraries are fine — they are not rhythm.
    re: /\b(?:m|p)[trblxye]?-\[|(?:gap|space-[xy])-\[/g,
    why:
      "arbitrary spacing — use the rhythm scale (multiples of the 8px beat), not a one-off pixel gap",
  },
];

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...walk(p));
    else if (/\.(svelte|ts|css|html)$/.test(name)) out.push(p);
  }
  return out;
}

const violations = [];
for (const file of walk(SRC)) {
  if (EXEMPT.includes(file)) continue;
  const text = readFileSync(file, "utf8");
  const lines = text.split("\n");
  for (const { re, why } of RULES) {
    lines.forEach((line, i) => {
      // Skip comment lines — a token name mentioned in prose is not a use.
      const trimmed = line.trim();
      if (
        trimmed.startsWith("//") ||
        trimmed.startsWith("*") ||
        trimmed.startsWith("/*")
      ) {
        return;
      }
      for (const m of line.matchAll(re)) {
        violations.push({
          file: relative(ROOT, file),
          line: i + 1,
          match: m[0],
          why,
        });
      }
    });
  }
}

if (violations.length === 0) {
  console.log("design-lint: clean — every surface is on the tokens.");
  process.exit(0);
}

console.error(
  `design-lint: ${violations.length} violation(s) — reach the tokens, not past them:\n`,
);
for (const v of violations) {
  console.error(
    `  ${v.file}:${v.line}  ${JSON.stringify(v.match)}\n    → ${v.why}`,
  );
}
process.exit(1);
