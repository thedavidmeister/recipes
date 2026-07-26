import type { StructuredStep } from "./types";

/**
 * Pure helpers over a recipe's step DAG (#74/#75/#76): formatting a timer, and
 * reading parallel-vs-sequential structure off the `after` edges. The timer
 * *machinery* (ticking, alerts, persistence below) is a page concern; these are the
 * pure parts the component and the page share.
 */

/**
 * A recipe's total-time estimate as a person reads it — or `null` when there is
 * nothing honest to say (#84).
 *
 * The number this formats is `recipes.total_seconds`: the critical path over the
 * same step DAG the rest of this module walks (#79). It is **not** exact and it is
 * **not** always known, and both are a display problem, so both are handled here
 * rather than at every call site:
 *
 * - **Unknown** (`null`, or a non-positive/garbled value) returns `null`, so the
 *   surface shows *nothing*. An un-read recipe is not an instant one, and "0 min"
 *   would be a confident lie about the case we know least about.
 * - **A lower bound.** Untimed steps ("until golden") contribute nothing to the
 *   path, so the real cook takes *at least* this long. Hence the trailing `+`:
 *   `23 min+` says at-least, where `23 min` would claim exact. Minutes are floored
 *   for the same reason — rounding up would overstate a bound already optimistic.
 *
 * Under an hour reads as minutes (`23 min+`); over it, as hours carrying the
 * remainder (`2 hours 5 min+`). Same "min"/"hour"/"hours" vocabulary the plan lobby
 * uses for its cap (#80), so one estimate is described the same way on both
 * surfaces. The remainder is not decoration: the corpus's long recipes almost never
 * land on a whole hour, and a real 9000-second braise reading "150 min+" is
 * arithmetic rather than an answer to "have I got time for this".
 */
export function formatEstimate(
  seconds: number | null | undefined,
): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const total = Math.floor(seconds);
  if (total < 60) return `${total} sec+`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes} min+`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  const label = hours === 1 ? "1 hour" : `${hours} hours`;
  return rest === 0 ? `${label}+` : `${label} ${rest} min+`;
}

/** A duration as a clock: `30:00`, `1:00`, `1:05:00`. Seconds always two digits. */
export function formatClock(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/**
 * Each step's depth in the DAG — the length of its longest dependency chain. Steps
 * that share a depth have no ordering between them, so they run in parallel. `after`
 * only references earlier steps (the backend validates this), so the recursion
 * terminates; a `visiting` guard defends against a malformed cycle anyway.
 */
export function stepDepths(steps: StructuredStep[]): Map<number, number> {
  const byId = new Map(steps.map((s) => [s.id, s]));
  const depth = new Map<number, number>();
  const visiting = new Set<number>();
  const of = (id: number): number => {
    const cached = depth.get(id);
    if (cached !== undefined) return cached;
    if (visiting.has(id)) return 0;
    visiting.add(id);
    const step = byId.get(id);
    // Guard `after` defensively: the backend validates it, but a malformed row must
    // degrade (treat the step as a root) rather than throw and blank the whole view.
    const d =
      step && Array.isArray(step.after) && step.after.length
        ? 1 + Math.max(...step.after.map(of))
        : 0;
    visiting.delete(id);
    depth.set(id, d);
    return d;
  };
  for (const step of steps) of(step.id);
  return depth;
}

/** A run of steps at the same depth — one step, or several that run in parallel. */
export interface StepGroup {
  depth: number;
  steps: StructuredStep[];
}

/**
 * The `cook` steps grouped by depth, in order — the method as a sequence of stages,
 * where a stage holding more than one step is a parallel group ("meanwhile"). Prep
 * (`kind: "prep"`) is rendered separately as mise en place, so it is excluded here.
 */
export function cookStages(steps: StructuredStep[]): StepGroup[] {
  const depths = stepDepths(steps);
  const byDepth = new Map<number, StructuredStep[]>();
  for (const step of steps) {
    if (step.kind !== "cook") continue;
    const d = depths.get(step.id) ?? 0;
    const bucket = byDepth.get(d) ?? [];
    bucket.push(step);
    byDepth.set(d, bucket);
  }
  return [...byDepth.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([depth, group]) => ({ depth, steps: group }));
}

// --- Running-timer persistence -------------------------------------------------
// A running timer is a deadline (ms). Persisting it per recipe means a reload — or a
// tab switch — mid-cook keeps the countdown, and a timer that finished while away
// shows as done rather than lost.

/** Live timer state for one step, as the component renders it. */
export interface StepTimer {
  remaining: number;
  done: boolean;
}

/** The persisted timers for one recipe: step id → deadline (unix ms). */
export type Deadlines = Record<number, number>;

function timersKey(source: string, id: string): string {
  return `recipes:cook-timers:${JSON.stringify([source, id])}`;
}

/** Restore a recipe's running timers. Absent/corrupt storage yields no timers. */
export function loadDeadlines(source: string, id: string): Deadlines {
  try {
    const raw = localStorage.getItem(timersKey(source, id));
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return {};
    }
    const out: Deadlines = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof v === "number" && Number.isFinite(v)) out[Number(k)] = v;
    }
    return out;
  } catch {
    return {};
  }
}

/** Persist a recipe's running timers so they survive a reload mid-cook. */
export function saveDeadlines(source: string, id: string, deadlines: Deadlines): void {
  try {
    localStorage.setItem(timersKey(source, id), JSON.stringify(deadlines));
  } catch {
    // No storage: timers just won't survive a reload.
  }
}
