//! **The app's shared framework for time-sensitive event handling** (#208).
//!
//! Not cook-timer plumbing that happens to be reusable: this is the one place a
//! session event is timed, authorised, applied and announced, and cook timers
//! ([`crate::timers`]) are merely its first consumer. Everything below is written in
//! terms of *an event* — nothing here knows what a timer is.
//!
//! ## Why a framework at all: whose clock is the truth
//!
//! A plan is a room of phones. When somebody taps "start the rice", three clocks are in
//! play — theirs, the server's, and every peer's — and they disagree, by seconds on a
//! good day and by minutes on a phone whose time zone was set by hand. Three obvious
//! answers are each wrong:
//!
//! - **Everyone renders off their own clock from their own start.** That is what cook
//!   did before this, and it is not a shared timer at all — it is N private ones.
//! - **The server stamps its receipt.** Then the event moves with the network: the same
//!   tap is a different instant on a good connection and on a train, and a phone that
//!   reconnects after a stall lands its tap wherever the socket got round to it.
//! - **The initiator's raw timestamp is stored as-is.** Then a phone forty minutes
//!   fast starts everyone's 30-minute simmer already finished.
//!
//! So (ruled, #208): **the initiator records the instant, and the room accepts it —
//! after it has been corrected for the initiator's own clock drift.** Each connection's
//! offset from the server timeline is measured over its socket ([`ClockOffset`]),
//! recorded for as long as that connection lives, and refreshed for as long as it lives
//! (sessions run days or weeks, #202, and clocks wander). An event's local timestamp is
//! normalised into the **shared timeline** on the way in ([`normalize`]); each client
//! translates shared instants back through its *own* recorded offset to render. The
//! result is that the countdown on every phone agrees with the moment of the tap,
//! whoever's clock is wrong.
//!
//! The **shared timeline is the server's clock** — not because the server is a better
//! clock, but because it is the one participant everybody already has a round trip to,
//! and the one that is still there when a phone goes away. Nothing here assumes it is
//! *correct*; it is the common reference the offsets are expressed against.
//!
//! ## The event path (one path, for every kind)
//!
//! ```text
//! client: {"type":"event","at":<local ms>,"event":{"kind":…}}
//!            │
//!            ├─ normalize   at → shared timeline, via this connection's ClockOffset
//!            ├─ guard       the kind's policy, applied here and only here
//!            ├─ apply       the kind's handler: one durable write
//!            └─ announce    the frames the handler produced, to the whole room
//! ```
//!
//! Each stage is framework code and each is asked once, so a new event kind supplies a
//! payload, a [`Guard`], and an `apply` arm — never its own copy of that choreography.
//!
//! ## Everything a session raises comes through here (#209)
//!
//! Cook timers were the first consumer; the events that predate the framework — a
//! **vote**, a **shopping tick**, the **pantry seed** it triggers and the **decision**
//! that a vote can complete — are the rest of them, and since #209 they arrive by this
//! path too. There is no second door: [`crate::session::socket_loop`] reads one client
//! frame kind that writes anything (`ClientMsg::Event`), and the shopping tick's old
//! `POST /api/session/{channel}/buy` is gone rather than kept beside it.
//!
//! The migration cost this module nothing, which is the claim #208 made and this is the
//! test of it: [`EventEnvelope`] is generic over its payload, [`normalize`] knows only
//! about instants, [`Guard`] names a predicate about a person and a plan rather than a
//! feature, and [`ingest`] carries whatever frames a handler returns. Each event that
//! moved supplied a [`SessionEvent`] variant, a guard (all of them want the one that
//! already existed) and an [`apply`] arm calling the *same* handler it always did — the
//! same SQL, the same predicates inside each write, the same frames to the room.
//!
//! What each of them **gained** is the fact this framework exists to carry: the instant
//! the initiator raised it, normalised through that participant's measured drift. A
//! swipe now knows when it was swiped rather than when its row was written; a tick knows
//! when the hand closed on the flour; and the decision knows the instant of the tap that
//! completed it, which is the only honest answer for an event nobody sends and everybody
//! caused (migration 0028).
//!
//! Two things deliberately did **not** move. The decision is not a wire kind — no client
//! may raise it, so it stays what #205 made it: a consequence evaluated inside the
//! deciding vote's own write. And the pantry seed is not one either: nobody taps a
//! pre-tick, so it runs where it always ran (the first time a list is asked for, by a
//! read or by a tick) and is stamped with the server's own clock, which *is* the shared
//! timeline.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use libsql::Connection;
use serde::{Deserialize, Serialize};

use crate::session::{is_seated_in_a_plan, is_seated_in_a_started_plan, ServerMsg};

/// The server clock, in unix milliseconds — **the shared timeline**.
///
/// Read here and nowhere else, so "what time is it" has one answer per process and a
/// test can reason about the arithmetic without one.
///
/// Milliseconds because a countdown is rendered to the second: quantising the anchor to
/// whole seconds (as the session tables' `unixepoch()` does) would be up to a second of
/// disagreement between two phones watching the same pot, which is visible. A clock
/// before 1970 is not a case worth carrying a `Result` for — it saturates to 0.
pub fn server_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---- clock drift -----------------------------------------------------------

/// How many round trips are kept per connection. Eight at the [`crate::session`]
/// keepalive's 30s cadence is four minutes of history: long enough that a single
/// stalled sample cannot be the whole estimate, short enough that a phone whose clock
/// was just corrected (a manual change, an NTP step, a flight landing in a new zone) is
/// re-learned within minutes rather than being averaged against its own past.
const WINDOW: usize = 8;

/// One measured round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sample {
    /// The full round trip, server-measured, in ms.
    rtt_ms: i64,
    /// How far ahead of the shared timeline the client's clock read: `client - server`.
    offset_ms: i64,
}

/// **One participant's clock, as the shared timeline sees it** — measured, recorded and
/// kept fresh for the life of a connection.
///
/// The measurement is the classic one, and the framework's, not any feature's: the
/// server sends its own reading, the client answers with that reading plus its own, and
/// the server times the return. With `t0` the send, `c` the client's reading and `t1`
/// the receipt,
///
/// ```text
/// rtt    = t1 - t0
/// offset = c - (t0 + rtt/2)      "the client's clock, minus the shared timeline"
/// ```
///
/// — halving the round trip is the assumption that the two legs took about as long as
/// each other. When they did not, the error is bounded by `rtt/2`, which is what makes
/// the **estimate the lowest-RTT sample in the window** rather than a mean of them.
/// Averaging drags a clean measurement toward a delayed one and has no bound at all; a
/// short round trip has little room to hide asymmetry in, so the best sample is the most
/// trustworthy one, and taking the best of a moving window *is* the smoothing. (This is
/// what NTP does, and for this reason.) A radio wake-up or a garbage-collection pause
/// inflates one leg by hundreds of milliseconds, and that sample is then simply not the
/// one used.
///
/// **Refresh cadence: every keepalive tick, ~30s** — the ping rides the frame
/// [`crate::session::socket_loop`] already sends to keep Render's 5-minute idle close
/// away, so drift tracking costs no extra wake-ups. Over a plan that runs for days that
/// is thousands of measurements; the window keeps the last eight.
///
/// Recorded **per connection, in memory**, never persisted: an offset describes a live
/// socket, and a reconnect (or a second phone) re-measures in one round trip. Persisting
/// it would mean serving a phone whose clock has since been corrected out of a record of
/// how wrong it used to be.
#[derive(Debug, Default)]
pub struct ClockOffset {
    samples: VecDeque<Sample>,
    /// The ping this connection is waiting on: the server reading it sent. A pong that
    /// does not echo it is dropped — a stale answer to a ping two ticks ago would be
    /// timed against the wrong send and read as a huge round trip.
    pending: Option<i64>,
}

impl ClockOffset {
    /// A connection with nothing measured yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a ping carrying `server_ms` has gone out. The previous unanswered
    /// ping is forgotten: an answer that never came is not evidence about the clock,
    /// and the next tick is 30 seconds away.
    pub fn ping_sent(&mut self, server_ms: i64) {
        self.pending = Some(server_ms);
    }

    /// Fold in the answer to a ping, returning the sample it produced.
    ///
    /// `echoed_ms` is the server reading the client sent back — it must be the ping this
    /// connection is waiting on, or there is nothing to time the round trip against and
    /// the frame is dropped. `client_ms` is the client's own clock at the moment it
    /// answered, and it is the only number in the exchange this side cannot check: a
    /// client that reports a false one moves *its own* offset, which is why the shared
    /// timeline is not the last line of defence — see [`normalize`]'s bound.
    pub fn pong(&mut self, echoed_ms: i64, client_ms: i64, received_ms: i64) -> bool {
        if self.pending != Some(echoed_ms) {
            return false;
        }
        self.pending = None;
        // A clock that went backwards between the send and the receipt (a step
        // correction on *this* box mid-flight) would read as a negative round trip;
        // floor it at zero rather than let it become a negative half-trip below.
        let rtt_ms = (received_ms - echoed_ms).max(0);
        let offset_ms = client_ms - (echoed_ms + rtt_ms / 2);
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample { rtt_ms, offset_ms });
        true
    }

    /// The current estimate: how far ahead of the shared timeline this client's clock
    /// reads, in ms.
    ///
    /// **`0` until something has been measured**, and "measured as synchronised" and
    /// "not measured yet" are genuinely different facts that this one number cannot
    /// tell apart. Zero is the only honest starting assumption — there is nothing else
    /// to guess — and it is exactly why [`normalize`] does not lean on this number
    /// alone: the sanity bound there is what covers the connection whose clock is wrong
    /// and has not been measured yet.
    pub fn offset_ms(&self) -> i64 {
        self.best().map(|s| s.offset_ms).unwrap_or(0)
    }

    /// The round trip the current estimate came from, in ms — the estimate's error bar,
    /// reported to the client so it can say how well it knows the shared timeline.
    pub fn rtt_ms(&self) -> i64 {
        self.best().map(|s| s.rtt_ms).unwrap_or(0)
    }

    /// The lowest-RTT sample in the window; ties go to the newer one, which is what
    /// keeps a days-long session tracking a wandering clock instead of pinning itself
    /// to one lucky measurement from Tuesday.
    fn best(&self) -> Option<Sample> {
        self.samples
            .iter()
            .copied()
            .reduce(|a, b| if b.rtt_ms <= a.rtt_ms { b } else { a })
    }
}

// ---- normalisation ---------------------------------------------------------

/// How far **before** the drift-compensated receipt a normalised instant may land.
///
/// Latency only ever delays an event, so a tap that reads as older than its receipt is
/// the ordinary case and the room is generous about it: a phone that stalls in a tunnel
/// and delivers on reconnect still lands its tap where the tap happened. Five minutes is
/// far past any real socket delay while still bounding what an unmeasured or lying
/// clock can do — a step whose duration is under five minutes cannot be pre-expired by
/// more than its own length.
const MAX_BACKDATE_MS: i64 = 5 * 60 * 1000;

/// How far **after** the receipt a normalised instant may land.
///
/// Nothing legitimate arrives from the future: the tap happened before the frame did.
/// The allowance is measurement error (half a round trip, plus whatever the offset
/// estimate is off by), not a window for the clock to be wrong in.
const MAX_AHEAD_MS: i64 = 30 * 1000;

/// **The initiator's instant, in the shared timeline.**
///
/// `local_ms` is the initiator's own clock at the moment of the tap and `offset` is
/// what this connection's clock has been measured to be doing, so `local - offset` is
/// where that tap sits on the timeline everybody shares. That subtraction is the whole
/// mechanism, and it is the reason a phone forty minutes fast still starts everyone's
/// timer at the right instant: its forty minutes are in the offset, and they cancel.
///
/// **`received_ms` is a bound, never the answer.** Clamping into
/// `[received - MAX_BACKDATE_MS, received + MAX_AHEAD_MS]` is what stops an unmeasured
/// or dishonest clock from starting a timer already finished (or in the middle of next
/// week) — but a sane event is never touched by it, so the receipt does not creep back
/// in as the truth. An event that *is* clamped is one whose clock we have reason to
/// disbelieve, and the edge of the plausible window is a better answer for it than the
/// receipt, because the receipt is the one thing we have already ruled out.
pub fn normalize(local_ms: i64, offset: &ClockOffset, received_ms: i64) -> i64 {
    let shared = local_ms.saturating_sub(offset.offset_ms());
    shared.clamp(
        received_ms.saturating_sub(MAX_BACKDATE_MS),
        received_ms.saturating_add(MAX_AHEAD_MS),
    )
}

// ---- the events -----------------------------------------------------------

/// **A session event, as a client raises it.**
///
/// The payload only — who raised it is the authenticated session and *when* rides on
/// the envelope, so neither is a field a client can fill in. A frame naming an initiator
/// would be a frame claiming to be somebody else.
///
/// Deliberately thin. A kind carries what the server cannot work out for itself and
/// nothing more: `TimerStart` names a step, and **no duration**, because the duration is
/// the recipe's and is read from the database ([`crate::timers::step_seconds`]) — the
/// initiator owns *when*, never *how long*. There is no field on this wire for one phone
/// to make a 30-minute step three seconds long for the room, and `deny_unknown_fields`
/// means a client that sends one is refused rather than quietly ignored: a frame that
/// looked accepted while its length was dropped is a phone counting down a number
/// nobody else has.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionEvent {
    /// Start (or restart) the shared countdown on one step of the plan's recipe.
    TimerStart {
        source: String,
        id: String,
        step: i64,
    },
    /// Take one step's shared countdown off the room's screens.
    TimerDismiss {
        source: String,
        id: String,
        step: i64,
    },
    /// **This client's yes or no on a recipe** (#209, formerly `{"type":"vote"}`).
    ///
    /// The voter is not here for the reason nobody else is: it is the authenticated
    /// session. What it gained by moving is `at` — the instant of the swipe itself,
    /// rather than the `unixepoch()` the row defaulted to whenever the write landed.
    ///
    /// It is also the event that can end the pick, because the win condition is
    /// evaluated inside this vote's own write (#205). That is not a second kind: no
    /// client raises a decision, and one that could would be a client deciding.
    Vote {
        source: String,
        id: String,
        vote: bool,
    },
    /// **A line of the meal's shopping list, claimed or put back** (#131/#209, formerly
    /// `POST /api/session/{channel}/buy`).
    ///
    /// `checked` carries both directions rather than there being two kinds, because that
    /// is what the tap is: one control with two states, and a claim whose release is a
    /// separate event kind is a claim whose release could be guarded differently.
    ///
    /// `index` is a position in the recipe's shopping-list projection, so a negative one
    /// names no line of any recipe. It is refused in [`crate::session::tick_item`]'s own
    /// predicate rather than here — the wire cannot express "a non-negative integer", and
    /// a check that lives anywhere but the write is a check a second caller can miss.
    BuyTick {
        source: String,
        id: String,
        index: i64,
        checked: bool,
    },
    /// **The room's soundtrack moves on** (#212) — one section's music starts, or the
    /// track that was playing has ended.
    ///
    /// One kind for both, because they are one act with one race in it: several devices
    /// arrive in a section at once, and several devices' tracks end at once, and in each
    /// case exactly one report may move the room while the rest accept what it chose.
    ///
    /// `after` is **the start instant of the state this device is answering** — the
    /// track it heard end, or `null` for "this section has no music yet". It is a value
    /// that came from the server in the first place, and the write is a compare-and-set
    /// on it ([`crate::music::advance`]), so a device that slept through two rollovers
    /// is refused rather than dragging the room back a song.
    ///
    /// **No track on the wire, deliberately**, and it is the [`SessionEvent::TimerStart`]
    /// rule with more at stake: the initiator owns *when*, the room owns *what*. A track
    /// name from a client is a URL every phone in the plan would load, and
    /// `deny_unknown_fields` refuses a frame carrying one instead of quietly ignoring it.
    MusicAdvance {
        section: crate::music::Section,
        after: Option<i64>,
    },
}

/// **What a kind requires of whoever raised it** — policy, named once per kind and
/// applied at the framework's one choke point ([`ingest`]).
///
/// A guard lives here rather than inside a handler so that adding an event kind is a
/// declaration of who may raise it, not a chance to forget to ask. It names a predicate
/// about a person and a plan, never a feature, which is what lets the existing events
/// migrate onto it unchanged: a vote, a tick and a timer all want exactly this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// On the roster of a plan whose swiping has begun — the predicate every shared
    /// write in this app already uses (`session::seated_in_a_started_plan`). Watchers
    /// (#180/#200) fail it, and so does a signed-in stranger holding the invite link.
    SeatedInStartedPlan,
    /// On the roster of a plan, **started or not** (`session::seated_in_a_plan`).
    ///
    /// The same people, one state earlier. Everything guarded above writes to the
    /// *outcome* of a meal — a vote, a claim on a shopping line, a countdown on a pot —
    /// and none of those may be written before the roster closes. The room's soundtrack
    /// (#212) is not an outcome: a lobby waiting for the host to start is a room, and it
    /// is hearing the music. So this variant moves exactly one line and not the one that
    /// matters: a watcher and a stranger with the link still fail it, which is the
    /// boundary #200 draws.
    SeatedInPlan,
}

impl SessionEvent {
    /// This kind's policy. Exhaustive on purpose: a new kind does not compile until
    /// somebody has said who may raise it.
    pub fn guard(&self) -> Guard {
        match self {
            // Every kind wants the same one, and that is the finding rather than a
            // shortcut: `seated_in_a_started_plan` is the predicate the vote, the tick
            // and the timer each hand-rolled before they arrived here, so migrating them
            // added no policy — it removed three copies of one (#175/#179/#209).
            SessionEvent::TimerStart { .. }
            | SessionEvent::TimerDismiss { .. }
            | SessionEvent::Vote { .. }
            | SessionEvent::BuyTick { .. } => Guard::SeatedInStartedPlan,
            // The one kind that is not about the meal's outcome, and so the one kind
            // the room has before the host starts it (#212). See `Guard::SeatedInPlan`.
            SessionEvent::MusicAdvance { .. } => Guard::SeatedInPlan,
        }
    }
}

/// **An event on the shared timeline**: what happened, who raised it, and when — the
/// three facts every consumer of this framework is handed.
///
/// Generic over the payload so the machinery around it is provably not about any one
/// feature; `SessionEvent` is simply the payload the socket deserialises today.
///
/// `at` is already normalised — an envelope cannot be built out of a raw local
/// timestamp, so no handler can accidentally use one. That is the invariant the whole
/// module exists to hold: everything downstream of [`ingest`] compares instants that are
/// all in the same timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnvelope<T> {
    /// The authenticated session that raised it — never a wire field.
    pub initiator: String,
    /// The plan it happened in.
    pub channel: String,
    /// When it happened, in the shared timeline: the initiator's own instant, corrected
    /// for the initiator's own clock drift.
    pub at: i64,
    /// What happened.
    pub payload: T,
}

/// **The choke point**: normalise, guard, apply, announce — for every event kind.
///
/// Returns the frames to broadcast to the room; an empty vector is "nothing was
/// recorded, so nothing is announced", which is the same rule `record_vote` follows and
/// for the same reason — a room told about a write the database refused would show a
/// countdown nobody can stop.
///
/// A refusal is silent, as every refusal on this socket is (#179/#180): a frame the
/// server never answers has nowhere to carry a reason. The client's half of that is not
/// offering the control — see the cook page's `watching`.
///
/// The guard is asked here **and** re-asserted inside each handler's own write. That is
/// not belt-and-braces, it is the #175/#179 discipline: this read tells the framework
/// whether to run a handler at all, and the write's own predicate is what makes the
/// answer race-free when a seat is given up in the round trip between them.
pub async fn ingest(
    conn: &Connection,
    channel: &str,
    initiator: &str,
    offset: &ClockOffset,
    local_ms: i64,
    received_ms: i64,
    payload: SessionEvent,
) -> anyhow::Result<Vec<ServerMsg>> {
    let event = EventEnvelope {
        initiator: initiator.to_owned(),
        channel: channel.to_owned(),
        at: normalize(local_ms, offset, received_ms),
        payload,
    };
    if !authorized(conn, &event).await? {
        return Ok(Vec::new());
    }
    apply(conn, &event).await
}

/// Whether the plan admits this event from this person, by the kind's own [`Guard`].
async fn authorized(
    conn: &Connection,
    event: &EventEnvelope<SessionEvent>,
) -> anyhow::Result<bool> {
    match event.payload.guard() {
        Guard::SeatedInStartedPlan => {
            is_seated_in_a_started_plan(conn, &event.channel, &event.initiator).await
        }
        Guard::SeatedInPlan => is_seated_in_a_plan(conn, &event.channel, &event.initiator).await,
    }
}

/// Hand an authorised event to its kind's handler and collect what the room is told.
///
/// The only per-kind code in the module, and it is a table: a handler performs one
/// durable write and answers with the frames that write produced, so ordering,
/// broadcast and the silence-on-refusal rule stay here rather than being re-decided per
/// feature.
async fn apply(
    conn: &Connection,
    event: &EventEnvelope<SessionEvent>,
) -> anyhow::Result<Vec<ServerMsg>> {
    match &event.payload {
        SessionEvent::TimerStart { source, id, step } => {
            crate::timers::start(
                conn,
                &event.channel,
                source,
                id,
                *step,
                &event.initiator,
                event.at,
            )
            .await?;
            announce_timers(conn, &event.channel, source, id).await
        }
        SessionEvent::TimerDismiss { source, id, step } => {
            crate::timers::dismiss(conn, &event.channel, source, id, *step, &event.initiator)
                .await?;
            announce_timers(conn, &event.channel, source, id).await
        }
        // The swipe, and the one thing a swipe can finish (#205/#209).
        //
        // **Announce nothing unless the row was written**, which is the rule this arm
        // brought with it from the socket loop rather than a new one: peers apply a vote
        // frame to their tally incrementally, so announcing a vote the database declined
        // would put a phantom yes on every open client until each next rehydrates.
        // `false` here is the write's own predicate refusing — not a decider, the
        // swiping has not begun, or the plan has already decided (`in_an_undecided_plan`,
        // which the framework's guard deliberately does not cover: shopping happens
        // *after* the decision, so one predicate for both would make the decision close
        // the shop it opened).
        //
        // The decision is asked **only on the vote that might have met it**, and only
        // when that vote was recorded — no poll, no sweep, no browser. It answers for
        // the call that recorded it and for no other, so two votes completing at once
        // produce exactly one `Decided`: the loser of that race changed no row and says
        // nothing.
        //
        // Worth stating, because a mutation pass finds it and a reader should not have
        // to: for **this** kind the choke point's guard is currently *equivalent* to the
        // write's own predicate. `record_vote` asks `seated_in_a_started_plan` and then
        // some, and this arm announces nothing when it answers `false`, so making
        // [`authorized`] pass a vote unconditionally changes no observable behaviour —
        // an equivalent mutation, in the sense `crate::session::decide_if_agreed`'s
        // redundant veto clause already names. It is not so for a timer or a tick, which
        // announce whether or not the write moved a row (see
        // `a_watcher_ticks_nothing_and_the_room_hears_nothing`), and the guard stays here
        // for all four because policy belongs on the framework: the next kind to arrive
        // gets asked whether it may run *before* anybody has remembered to put the
        // predicate inside its write.
        SessionEvent::Vote { source, id, vote } => {
            if !crate::session::record_vote(
                conn,
                &event.channel,
                source,
                id,
                &event.initiator,
                *vote,
                event.at,
            )
            .await?
            {
                return Ok(Vec::new());
            }
            let mut frames = vec![ServerMsg::Vote {
                voter: event.initiator.clone(),
                source: source.to_owned(),
                id: id.to_owned(),
                vote: *vote,
            }];
            if let Some(d) =
                crate::session::decide_if_agreed(conn, &event.channel, source, id, event.at).await?
            {
                frames.push(ServerMsg::Decided {
                    source: d.source,
                    id: d.id,
                    decided_at: d.decided_at,
                });
            }
            Ok(frames)
        }
        // A line of the shopping list, taken or put back (#131/#209).
        //
        // The **seed runs first** (#156), exactly as it did when this was an HTTP
        // handler and for the same reason: a tick can be the first thing that ever
        // touches this list — somebody opened `buy` on a peer's phone and tapped before
        // this client read it — so a first tap has to land *on top of* the pantry's
        // answer rather than be overwritten by a seed arriving after it.
        //
        // Unlike the vote, the list is announced **whether or not the write changed a
        // row**, which is again what this arm inherited: the old handler re-read and
        // announced unconditionally after the write. It is also the right rule
        // ([`announce_timers`] gives it) — a whole-list frame is self-healing, so a tick
        // the write predicate refused corrects the tapper's screen with the truth
        // instead of leaving it showing a claim nobody holds.
        SessionEvent::BuyTick {
            source,
            id,
            index,
            checked,
        } => {
            crate::session::seed_buy_list(conn, &event.channel, source, id).await?;
            if *checked {
                crate::session::tick_item(
                    conn,
                    &event.channel,
                    source,
                    id,
                    *index,
                    &event.initiator,
                    event.at,
                )
                .await?;
            } else {
                crate::session::untick_item(
                    conn,
                    &event.channel,
                    source,
                    id,
                    *index,
                    &event.initiator,
                )
                .await?;
            }
            announce_buy(conn, &event.channel, source, id).await
        }
        // The room's soundtrack moves on (#212).
        //
        // Announced **whether or not the write moved the row**, which is
        // [`announce_timers`]' rule on the state it matters most for: several devices
        // report the same rollover, one wins the compare-and-set, and this frame is how
        // every loser is told what the winner chose. A refusal answered with silence
        // would leave each of them playing a track nobody else in the room is on — the
        // exact desync this kind exists to close.
        //
        // A section with no tracks (`joy`, today) announces nothing, because there is no
        // state to state — not a frame claiming silence.
        SessionEvent::MusicAdvance { section, after } => {
            crate::music::advance(
                conn,
                &event.channel,
                *section,
                &event.initiator,
                *after,
                event.at,
            )
            .await?;
            announce_music(conn, &event.channel, *section).await
        }
    }
}

/// One section's soundtrack, as a frame — [`announce_timers`]' whole-state rule, on the
/// room's music (#212).
async fn announce_music(
    conn: &Connection,
    channel: &str,
    section: crate::music::Section,
) -> anyhow::Result<Vec<ServerMsg>> {
    Ok(crate::music::current(conn, channel, section)
        .await?
        .map(|t| ServerMsg::Music {
            section: t.section,
            track: t.track,
            started_at: t.started_at,
        })
        .into_iter()
        .collect())
}

/// The room's whole shopping checklist for one recipe, as a frame — [`announce_timers`]'
/// rule, on the table it was written for (#131).
async fn announce_buy(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<Vec<ServerMsg>> {
    Ok(vec![ServerMsg::Buy {
        source: source.to_owned(),
        id: id.to_owned(),
        checks: crate::session::load_buy_checks(conn, channel, source, id).await?,
    }])
}

/// The room's whole timer state for one recipe, as a frame.
///
/// **Whole, never a delta** — the [`ServerMsg::Buy`] rule (#131): a client that missed a
/// frame is corrected by the next one instead of drifting, and two people tapping at
/// once have no ordering to get wrong. Sent even when the write changed nothing (a
/// dismiss of a timer that was already gone), because re-stating the truth is the
/// cheapest possible way to be right.
async fn announce_timers(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<Vec<ServerMsg>> {
    Ok(vec![ServerMsg::Timers {
        source: source.to_owned(),
        id: id.to_owned(),
        timers: crate::timers::load(conn, channel, source, id).await?,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A connection that has measured one clean round trip: `rtt` there and back, with
    /// the client's clock `offset` ahead of the shared timeline.
    fn measured(rtt: i64, offset: i64) -> ClockOffset {
        let mut c = ClockOffset::new();
        let t0 = 1_000_000;
        c.ping_sent(t0);
        // What such a client would honestly answer: its clock at the moment the ping
        // reached it, half a round trip after the send.
        assert!(c.pong(t0, t0 + rtt / 2 + offset, t0 + rtt));
        c
    }

    /// The measurement itself: a symmetric round trip recovers the offset exactly, and
    /// reports the round trip it came from.
    #[test]
    fn a_round_trip_measures_the_client_clock() {
        let c = measured(40, 5_000);
        assert_eq!(c.offset_ms(), 5_000, "the client is 5s ahead");
        assert_eq!(c.rtt_ms(), 40);
        assert_eq!(c.samples.len(), 1);
    }

    /// A clock that agrees with the server measures as agreeing — an offset of 0 has to
    /// be *earned*, since an unmeasured connection reports the same number and only the
    /// sample behind it tells the two apart.
    #[test]
    fn an_unmeasured_connection_is_not_a_synchronised_one() {
        let fresh = ClockOffset::new();
        assert_eq!(fresh.offset_ms(), 0);
        assert!(fresh.samples.is_empty(), "nothing has been measured");

        let c = measured(40, 0);
        assert_eq!(c.offset_ms(), 0);
        assert_eq!(c.samples.len(), 1, "and this one has");
    }

    /// The window keeps the **lowest-RTT** sample, not the newest and not the mean.
    ///
    /// A stalled round trip carries up to `rtt/2` of asymmetry error, so a 4-second
    /// sample must not be allowed to move an estimate a 20ms one already made. Averaging
    /// would put the estimate ~1s off here; taking the best sample keeps it exact.
    #[test]
    fn a_stalled_sample_does_not_move_the_estimate() {
        let mut c = ClockOffset::new();
        c.ping_sent(1_000);
        assert!(c.pong(1_000, 1_010 + 7_000, 1_020)); // 20ms round trip, 7s ahead
        assert_eq!(c.offset_ms(), 7_000);

        // The same clock, measured over a round trip that stalled 4s on the way back:
        // the client answered honestly, but halving *this* trip mis-attributes 2s of it.
        c.ping_sent(2_000);
        assert!(c.pong(2_000, 2_010 + 7_000, 6_000));
        assert_eq!(
            c.offset_ms(),
            7_000,
            "the clean sample still holds the estimate"
        );
        assert_eq!(
            c.rtt_ms(),
            20,
            "and the estimate's error bar is that trip's"
        );
    }

    /// An asymmetric trip is *not* discarded when it is all there is — a bad estimate
    /// bounded by rtt/2 beats no estimate, which would be a silent offset of 0.
    #[test]
    fn one_asymmetric_sample_is_still_an_estimate() {
        let mut c = ClockOffset::new();
        c.ping_sent(1_000);
        // Out in 900ms, back in 100ms: the client's honest reading is t0+900, and
        // halving the 1000ms trip attributes 500 of it, so the estimate reads 400ms
        // ahead of a clock that is in fact synchronised. Bounded by rtt/2, as documented.
        assert!(c.pong(1_000, 1_900, 2_000));
        assert_eq!(c.offset_ms(), 400);
        assert!(c.offset_ms().abs() <= 1_000 / 2, "the error bar holds");
    }

    /// Drift is re-learned: a clock that is corrected mid-session stops being described
    /// by how wrong it used to be, once its old samples age out of the window.
    #[test]
    fn the_window_tracks_a_clock_that_moves() {
        let mut c = ClockOffset::new();
        for i in 0..WINDOW as i64 {
            let t0 = 1_000 + i * 30_000;
            c.ping_sent(t0);
            assert!(c.pong(t0, t0 + 10 + 60_000, t0 + 20));
        }
        assert_eq!(c.offset_ms(), 60_000, "a phone a minute fast");

        // The user fixes their clock. Eight refreshes later nothing from before it is
        // left in the window.
        for i in 0..WINDOW as i64 {
            let t0 = 900_000 + i * 30_000;
            c.ping_sent(t0);
            assert!(c.pong(t0, t0 + 10, t0 + 20));
        }
        assert_eq!(c.offset_ms(), 0, "and now it is right");
    }

    /// A pong that does not answer the outstanding ping is dropped: timing it against
    /// the wrong send would read as a round trip of whatever the tick interval is.
    #[test]
    fn a_pong_for_a_ping_we_did_not_send_is_dropped() {
        let mut c = ClockOffset::new();
        c.ping_sent(1_000);
        assert!(!c.pong(999_999, 1_000_000, 1_020), "not the ping we sent");
        assert!(c.samples.is_empty());

        assert!(c.pong(1_000, 1_010, 1_020));
        assert!(
            !c.pong(1_000, 1_010, 9_999),
            "and it is not answerable twice"
        );
        assert_eq!(c.rtt_ms(), 20);
    }

    /// **The point of the whole module.** A participant whose clock is wildly wrong
    /// lands their event at the same shared instant as one whose clock is right.
    #[test]
    fn a_wrong_clock_lands_the_event_at_the_right_instant() {
        let received = 1_700_000_000_000;
        // Two phones tap at the same real moment, 120ms before the frames arrive.
        let right = measured(40, 0);
        let wrong = measured(40, 42 * 60 * 1000); // 42 minutes fast

        let a = normalize(received - 120, &right, received);
        let b = normalize(received - 120 + 42 * 60 * 1000, &wrong, received);
        assert_eq!(a, b, "same tap, same instant on the shared timeline");
        assert_eq!(a, received - 120, "and it is the tap, not the receipt");
    }

    /// **Receipt latency does not move the event.** The same tap delivered promptly and
    /// delivered a second and a half late is one instant.
    #[test]
    fn latency_between_the_tap_and_the_receipt_does_not_shift_it() {
        let base = 1_700_000_000_000;
        let c = measured(40, 0);
        let prompt = normalize(base, &c, base + 30);
        let slow = normalize(base, &c, base + 1_500);
        assert_eq!(prompt, slow);
        assert_eq!(prompt, base);
    }

    /// The sanity bound catches what the offset cannot: a clock nobody has measured
    /// (or one lying about itself) cannot pre-expire a timer by an hour.
    #[test]
    fn an_unmeasured_wrong_clock_is_bounded_not_believed() {
        let received = 1_700_000_000_000;
        let unmeasured = ClockOffset::new();

        let hour_early = normalize(received - 60 * 60 * 1000, &unmeasured, received);
        assert_eq!(hour_early, received - MAX_BACKDATE_MS, "clamped, not taken");

        let week_late = normalize(received + 7 * 24 * 60 * 60 * 1000, &unmeasured, received);
        assert_eq!(week_late, received + MAX_AHEAD_MS);
    }

    /// And the bound does not quietly become "the server's receipt is the truth": an
    /// ordinary tap, however delayed, passes through it untouched.
    #[test]
    fn the_bound_leaves_an_ordinary_tap_alone() {
        let received = 1_700_000_000_000;
        let c = measured(40, 0);
        for delay in [0, 200, 5_000, 60_000, MAX_BACKDATE_MS - 1] {
            assert_eq!(
                normalize(received - delay, &c, received),
                received - delay,
                "a tap {delay}ms before its receipt is that tap"
            );
        }
    }

    /// Policy is declared per kind and lives on the framework, not in a handler.
    #[test]
    fn every_kind_names_its_guard() {
        let start = SessionEvent::TimerStart {
            source: "themealdb".into(),
            id: "52795".into(),
            step: 7,
        };
        let dismiss = SessionEvent::TimerDismiss {
            source: "themealdb".into(),
            id: "52795".into(),
            step: 7,
        };
        assert_eq!(start.guard(), Guard::SeatedInStartedPlan);
        assert_eq!(dismiss.guard(), Guard::SeatedInStartedPlan);
    }

    /// **The one kind whose policy is not the others'** (#212). Music is the room's
    /// atmosphere rather than the meal's outcome, so it is the only thing on this socket
    /// a lobby may raise — and the line it does *not* move is the one that matters: a
    /// watcher fails `SeatedInPlan` exactly as it fails `SeatedInStartedPlan`.
    #[test]
    fn the_soundtrack_is_the_room_s_before_the_plan_starts() {
        let advance = SessionEvent::MusicAdvance {
            section: crate::music::Section::Buy,
            after: None,
        };
        assert_eq!(advance.guard(), Guard::SeatedInPlan);
        assert_ne!(advance.guard(), Guard::SeatedInStartedPlan);
    }

    /// **No duration on the wire.** The initiator owns *when*; the recipe owns *how
    /// long*. A frame carrying `seconds` is refused as an unknown field rather than
    /// quietly ignored, so a client cannot believe it set one.
    #[test]
    fn the_wire_has_nowhere_to_put_a_duration() {
        let ok: SessionEvent = serde_json::from_str(
            r#"{"kind":"timer_start","source":"themealdb","id":"52795","step":7}"#,
        )
        .unwrap();
        assert_eq!(
            ok,
            SessionEvent::TimerStart {
                source: "themealdb".into(),
                id: "52795".into(),
                step: 7
            }
        );

        // Serialising a start round-trips to a frame with no duration in it at all.
        let wire = serde_json::to_string(&ok).unwrap();
        assert!(!wire.contains("seconds"), "{wire}");
        assert!(!wire.contains("duration"), "{wire}");

        // And a client that tries anyway is refused rather than quietly ignored
        // (`deny_unknown_fields`): silently dropping it would leave a phone believing
        // it had set a length while the room counted down a different one.
        let refused = serde_json::from_str::<SessionEvent>(
            r#"{"kind":"timer_start","source":"themealdb","id":"52795","step":7,"seconds":3}"#,
        );
        assert!(refused.is_err(), "{refused:?}");
    }
}
