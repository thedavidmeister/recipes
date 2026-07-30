#!/usr/bin/env node
/**
 * The `$env` value-import fence (#176/7).
 *
 * `$env/dynamic/public` is read at *runtime*, out of the SvelteKit runtime that only
 * the app has. Storybook and the unit runner do not have it — deliberately, in the
 * unit runner's case (see `vitest.config.ts`) — so importing a **value** from a module
 * that reads `$env` drags that read into a bundle where it is `undefined`, and every
 * story of the component dies on `Cannot read properties of undefined (reading 'env')`.
 *
 * Nothing else catches it. `svelte-check` is happy (the types are fine), `lint:design`
 * is happy (no colours involved), `npm run build` is happy (the *app* bundle does have
 * the runtime), and the visual fence photographs the crash without complaining, because
 * a crashed story still renders something. The one signal is a human opening Storybook.
 *
 * `$lib/shopping` exists to prevent exactly this — the pure half a story may import,
 * split from the `$lib/buy` half that keeps the fetches — so this lint is that split,
 * enforced. It is transitive, because the hazard is: a story imports a component, the
 * component imports a helper, the helper imports `$lib/buy`, and nothing in the diff
 * that added the helper mentions `$env` at all.
 *
 * Zero dependencies, like the lints beside it: a gate that needs its own build step is
 * a gate people disable.
 *
 * Checks every `*.stories.ts` and every `*.test.ts` under `src/`, following only
 * **value** imports (an `import type` is erased before it can run, and is the correct
 * way to reach `$lib/pick` for a `Voter`).
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const SRC = join(ROOT, "src");

/** Files whose *own* import of a SvelteKit runtime module makes them unusable here. */
const RUNTIME_ONLY = /^\$env\//;

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...walk(p));
    else if (/\.(svelte|ts)$/.test(name)) out.push(p);
  }
  return out;
}

/**
 * Strip comments so prose about `$env` — which `$lib/shopping` has a lot of, and
 * rightly — is never mistaken for an import of it.
 */
function code(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .filter((l) => !l.trim().startsWith("//"))
    .join("\n");
}

/**
 * Every module specifier this file imports, with whether the import survives to
 * runtime. `import type {...}` / `export type {...}` are erased; everything else —
 * a default import, a namespace, a side-effect `import "x"`, a dynamic `import("x")`,
 * and a brace list with even one non-`type` specifier — runs.
 *
 * An all-inline-type list (`import { type X } from "y"`) is counted as a value import
 * on purpose. Whether a bundler erases it depends on settings this lint does not read,
 * and the safe side of that guess is the side that keeps a crash out of Storybook.
 */
function imports(text) {
  const src = code(text);
  const found = [];
  for (
    const m of src.matchAll(
      /\b(import|export)\s+(type\s+)?([^;]*?)\s+from\s*["']([^"']+)["']/g,
    )
  ) {
    found.push({ spec: m[4], value: !m[2] });
  }
  for (const m of src.matchAll(/(?:^|\n)\s*import\s*["']([^"']+)["']/g)) {
    found.push({ spec: m[1], value: true });
  }
  for (const m of src.matchAll(/\bimport\s*\(\s*["']([^"']+)["']\s*\)/g)) {
    found.push({ spec: m[1], value: true });
  }
  return found;
}

/** A specifier resolved to a file in this tree, or `null` for anything outside it. */
function resolve(spec, from) {
  let base;
  if (spec.startsWith("$lib/")) {
    base = join(SRC, "lib", spec.slice("$lib/".length));
  } else if (spec.startsWith("./") || spec.startsWith("../")) {
    base = join(dirname(from), spec);
  } else return null;

  for (
    const candidate of [
      base,
      `${base}.ts`,
      `${base}.js`,
      `${base}.svelte`,
      join(base, "index.ts"),
      join(base, "index.js"),
      join(base, "index.svelte"),
    ]
  ) {
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

const files = walk(SRC);

// The graph: for each file, the files it reaches by a value import, plus the `$env`
// module it reads itself (if any).
const edges = new Map();
const reads = new Map();
for (const file of files) {
  const parsed = imports(readFileSync(file, "utf8"));
  const runtime = parsed.find((i) => i.value && RUNTIME_ONLY.test(i.spec));
  if (runtime) reads.set(file, runtime.spec);
  edges.set(
    file,
    parsed
      .filter((i) => i.value)
      .map((i) => resolve(i.spec, file))
      .filter((f) => f !== null),
  );
}

/**
 * The chain from `file` to a module that reads `$env`, or `null` if it reaches none.
 * The chain is the whole point of the report: the file that has to change is usually
 * not the file that reads `$env`, and not the story either.
 */
function chainToEnv(file, seen = new Set()) {
  if (seen.has(file)) return null;
  seen.add(file);
  if (reads.has(file)) return [file];
  for (const next of edges.get(file) ?? []) {
    const rest = chainToEnv(next, seen);
    if (rest) return [file, ...rest];
  }
  return null;
}

// Storybook and the unit runner are the two bundles with no SvelteKit around them.
const entries = files.filter((f) => /\.(stories|test)\.ts$/.test(f));

const violations = [];
for (const entry of entries) {
  const chain = chainToEnv(entry);
  if (chain) violations.push({ entry, chain });
}

if (violations.length === 0) {
  console.log(
    `env-fence: clean — none of the ${entries.length} story/test entry points ` +
      `value-imports a module that reads $env.`,
  );
  process.exit(0);
}

console.error(
  `env-fence: ${violations.length} entry point(s) drag $env into a bundle that has ` +
    `no SvelteKit runtime — every story of the component will crash on ` +
    `\`undefined (reading 'env')\`, and the visual fence will photograph it:\n`,
);
for (const { chain } of violations) {
  // The chain starts at the entry point itself, so it is the whole report — the file
  // to change is usually neither end of it.
  for (const [i, step] of chain.entries()) {
    const arrow = i === 0 ? "  " : `  ${"  ".repeat(i)}→ `;
    const tail = reads.has(step) ? `  (reads ${reads.get(step)})` : "";
    console.error(`${arrow}${relative(ROOT, step)}${tail}`);
  }
  console.error(
    `    → import the value from the pure half instead (the $lib/shopping ` +
      `vs $lib/buy split), or make it \`import type\` if it is only a type.\n`,
  );
}
process.exit(1);
