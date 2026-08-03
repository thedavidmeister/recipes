import type { PickHandlers, ServerMsg } from "./pick";

/**
 * Hand one room frame to the handler it belongs to (#20) — the socket's whole reading
 * of the wire, split out from the socket.
 *
 * Its own module for the `$lib/shopping` reason (#176/7, enforced by `lint:env`):
 * `$lib/pick` reaches `$lib/client`, which reads `$env/dynamic/public`, so importing a
 * *value* from it drags that read into the unit runner where it is `undefined`. The
 * types come back the other way as `import type`, which is erased, so nothing circular
 * survives to runtime.
 *
 * It is split out because of what a mistake here looks like. A frame this does not
 * recognise is dropped in silence — that is deliberate, and it is what lets one room
 * serve the pick and `buy` without either writing empty functions for the other's
 * traffic — so a branch that never fires is indistinguishable from a server that never
 * sent one. `svelte-check` is happy either way; a story renders either way; the socket
 * reconnects either way. Since #201 that silence has real weight: `decided` is the one
 * frame that ends a pick, and losing it strands whoever was not watching when the last
 * yes landed on a deck that is over. So the reading is a pure function with a test on
 * it, rather than a chain of `else if`s inside a class nothing can construct.
 *
 * The event framework's two clock frames (#208) are read here like any other, but the
 * handlers that answer them come from `PickClient` rather than from a page: keeping the
 * drift measurement going is the socket's business, and a page that had to remember to
 * answer a ping is a page that could forget and then render a countdown off a clock
 * nobody measured.
 */
export function applyFrame(msg: ServerMsg, handlers: PickHandlers): void {
  if (msg.type === "tally") {
    handlers.onTally?.(msg.participants, msg.votes);
  } else if (msg.type === "lobby") {
    handlers.onLobby?.(msg.deciders, msg.started);
  } else if (msg.type === "vote") {
    handlers.onVote?.(msg.voter, msg.source, msg.id, msg.vote);
  } else if (msg.type === "buy") {
    handlers.onBuy?.(msg.source, msg.id, msg.checks);
  } else if (msg.type === "left") {
    handlers.onLeft?.(msg.voter, msg.ended);
  } else if (msg.type === "decided") {
    handlers.onDecided?.({
      source: msg.source,
      id: msg.id,
      decided_at: msg.decided_at,
    });
  } else if (msg.type === "time_ping") {
    handlers.onTimePing?.(msg.server_ms);
  } else if (msg.type === "time_sync") {
    handlers.onTimeSync?.(msg.offset_ms, msg.rtt_ms);
  } else if (msg.type === "timers") {
    handlers.onTimers?.(msg.source, msg.id, msg.timers);
  }
}
