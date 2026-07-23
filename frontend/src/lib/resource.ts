import { createQuery } from "@tanstack/svelte-query";

/**
 * One place for "how a page turns a request into something a component can render",
 * and one place to see every screen that does it differently.
 *
 * Every page here follows the same split: the page owns the query, the component owns
 * rendering and takes `status` as a prop. That meant every page also wrote the same
 * six lines to get from one to the other — which is how `retry: false` ended up
 * declared twelve separate times, each copied from a comment that was right where it
 * was first written and wrong everywhere it was pasted. A shared default only helps if
 * there is a shared place for it to live.
 *
 * The deviations are listed at the bottom on purpose. A helper that quietly covers 80%
 * of cases leaves the other 20% invisible until someone changes the helper and breaks
 * them; naming them here means the whole picture is in one file even where the code
 * cannot be.
 */

/** What a component needs to know about a request. Mirrors the per-domain aliases
 * (`KitchensStatus`, `PlanStatus`) — same three words, so they interchange freely. */
export type LoadStatus = "pending" | "error" | "ready";

/**
 * What a page passes in: the query's own options, typed so the shape of the answer
 * flows from `queryFn` to the component without anyone restating it.
 */
export type ResourceOptions<T> = {
  queryKey: readonly unknown[];
  queryFn: () => Promise<T>;
} & Record<string, unknown>;

/** A request, as a page hands it to a component. */
export interface Resource<T> {
  readonly status: LoadStatus;
  readonly data: T | undefined;
  /** The failure in words, ready to show — not the Error itself. */
  readonly error: string | undefined;
  /** The underlying query, for the pages that need `refetch` or the raw flags. */
  readonly query: ReturnType<typeof createQuery>;
}

/**
 * A query, plus the derivation every page was writing by hand.
 *
 * Returns getters rather than values: destructuring a rune's state freezes it, so the
 * fields have to be read at the moment they are used or the page stops updating.
 */
export function resource<T>(options: () => ResourceOptions<T>): Resource<T> {
  const query = createQuery(options as Parameters<typeof createQuery>[0]);
  return {
    get status(): LoadStatus {
      return query.isError ? "error" : query.isPending ? "pending" : "ready";
    },
    get data(): T | undefined {
      return query.data as T | undefined;
    },
    get error(): string | undefined {
      return query.error instanceof Error ? query.error.message : undefined;
    },
    get query() {
      return query;
    },
  };
}

/**
 * Several requests behind one screen.
 *
 * Pending until everything has arrived, failed if anything failed, and the reported
 * failure is the first one — because a screen shows one message, and the first thing
 * that went wrong is usually the thing that caused the rest.
 */
export function together(...parts: Resource<unknown>[]) {
  return {
    get status(): LoadStatus {
      if (parts.some((p) => p.status === "error")) return "error";
      if (parts.some((p) => p.status === "pending")) return "pending";
      return "ready";
    },
    get error(): string | undefined {
      return parts.find((p) => p.error)?.error;
    },
  };
}

/**
 * The auth gate's version of the same derivation.
 *
 * Identical in shape and different in vocabulary, deliberately: "ready" is the wrong
 * word for "nobody is logged in", so the gate says `checking | idle | error` and the
 * login screen reads better for it. It lives here rather than in the layout because it
 * is the same three branches — if the meaning of "pending" ever changes, both should
 * change together.
 */
export function loginStatus(query: {
  isError: boolean;
  isPending: boolean;
}): "checking" | "idle" | "error" {
  return query.isError ? "error" : query.isPending ? "checking" : "idle";
}

/**
 * The two screens whose state does not live here, and why not.
 *
 * Both were considered for this file and left out on the same principle: a module that
 * turns requests into renderable state should not learn what an admin is, or what a
 * WebSocket is, to serve one caller each. Naming them here keeps the whole picture
 * visible from the thing they are exceptions to.
 *
 * - **The pick room** (`/pick/[channel]`) does not derive its state from a request at
 *   all. Its status comes from a socket — connecting, open, reconnecting — and from
 *   whether a deck has cards yet, which no query flag describes.
 *
 * - **The health dashboard** has a `forbidden` branch that depends on whether the
 *   viewer is an admin and on a 403 from the poll. Its two queries use {@link resource}
 *   like everywhere else; only that branch stays local, because it is the one part that
 *   is about admin-ness rather than about loading. It also shows both failures
 *   separately rather than through {@link together} — the point of that page is saying
 *   *which* half is broken.
 */
