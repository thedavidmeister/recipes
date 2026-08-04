//! Pick (#20): a live, shared swipe/vote over the corpus.
//!
//! People in a pick swipe yes/no through recipe cards; the app tallies the yeses
//! into "winners" — what to cook. Everyone walks the corpus **independently**
//! (their own order), but a vote **cross-pollinates**:
//! when anyone votes a recipe it is broadcast to the room, and every peer's client
//! silently slips that recipe into its own deck. So the group diverges for
//! discovery yet converges on every candidate anyone surfaced — which is what makes
//! the tally meaningful (everyone gets a shot at each voted recipe).
//!
//! **Turso is the source of truth; the WS room is only a live-push accelerator.**
//! Every vote is written to `votes` *and* broadcast over the room. A (re)joining
//! client — a late joiner, a 5-min-idle reconnect, or a reconnect after Render's
//! 15-min spin-down wiped every in-memory room — recovers the same way: read the
//! tally from Turso, then listen. A lost process is a performance blip, not data
//! loss.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::{Extension, Json};
use futures_util::{SinkExt, StreamExt};
use libsql::Connection;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

/// The live rooms: one broadcast channel per session, keyed by channel id, each
/// carrying JSON-serialized [`ServerMsg`] frames. Shared (cloned) in [`AppState`];
/// losing the map to a process restart is recovered from Turso on reconnect.
pub type Rooms = Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>;

/// A fresh, empty room registry for [`AppState`].
pub fn rooms() -> Rooms {
    Arc::new(Mutex::new(HashMap::new()))
}

/// The broadcast sender for `channel`, created on first join. The lock is held only
/// for the map lookup — never across an `.await`.
fn room(rooms: &Rooms, channel: &str) -> broadcast::Sender<String> {
    rooms
        .lock()
        .expect("rooms mutex poisoned")
        .entry(channel.to_string())
        .or_insert_with(|| broadcast::channel(256).0)
        .clone()
}

fn mint_channel_id() -> String {
    let mut buf = [0u8; 8];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// **A plan's seed** — the one number all of its shared randomness dangles off (#212).
///
/// Not a music number. Anything a room has to agree about that nobody decides — which
/// song is playing, and whatever comes next — is a pure function of this and how long the
/// plan has existed, so there is no state to write, no authority to establish and nothing
/// to rehydrate. A device that has been asleep for an hour computes the same answer as
/// one that has been watching all along.
///
/// **Below 2^53**, and that bound is the contract rather than an implementation detail:
/// every consumer is a browser, `Number` is a double, and 2^53 is the last integer a
/// double holds exactly. A wider seed would sometimes arrive as a *different* number on
/// two phones, which is the disagreement the seed exists to remove. 53 bits from the OS
/// CSPRNG is far more than the choices it feeds need.
fn mint_seed() -> i64 {
    let mut buf = [0u8; 8];
    OsRng.fill_bytes(&mut buf);
    (i64::from_be_bytes(buf) & ((1 << 53) - 1)).abs()
}

// ---- WS protocol -----------------------------------------------------------

/// A frame from a client.
///
/// **Two kinds, and only one of them writes anything** (#209). A vote used to be its
/// own `{"type":"vote"}` frame here; it is now a [`crate::events::SessionEvent::Vote`]
/// payload inside [`ClientMsg::Event`], which is what "one choke point" means at the
/// wire: there is exactly one frame a client can send that changes the plan, and it
/// carries the instant the client did it.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientMsg {
    /// The client's half of a clock measurement — its answer to [`ServerMsg::TimePing`].
    ///
    /// `server_ms` is the reading the ping carried, echoed back so the round trip is
    /// timed against the send it belongs to; `client_ms` is the client's own clock when
    /// it answered. The framework folds the pair into this connection's recorded offset
    /// (`events::ClockOffset`), which is what every event from it is normalised through.
    TimePong { server_ms: i64, client_ms: i64 },
    /// **An event, through the framework** ([`crate::events`]) — the one path a
    /// time-sensitive action takes.
    ///
    /// `at` is the **initiator's own clock at the moment of the action**, and it is the
    /// event's instant: the tap is what happened, not the arrival of the frame carrying
    /// it. It is normalised into the shared timeline through this connection's recorded
    /// offset before anything downstream sees it, so a phone whose clock is wrong still
    /// lands its event where the room agrees it happened.
    ///
    /// Who raised it is never on the wire — it is the authenticated session, like every
    /// other write on this socket.
    Event {
        at: i64,
        event: crate::events::SessionEvent,
    },
}

/// A frame to a client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ServerMsg {
    /// The full tally, sent on join so a (re)connecting client rehydrates before
    /// listening.
    ///
    /// `participants` is the distinct-voter count — how many people have swiped at
    /// all. It is **not** the number a recipe has to win over, and consensus is not
    /// evaluated against it (#181): one person's first yes arrives here as
    /// `participants: 1, yes: 1, no: 0`, which is unanimous by this count alone.
    ///
    /// Since #201 nothing on either side of the wire decides anything against *either*
    /// count. The win condition is evaluated here, against the roster, inside the
    /// vote's own write, and its answer arrives as [`ServerMsg::Decided`]. This is the
    /// running score, for display.
    Tally {
        participants: i64,
        votes: Vec<TallyRow>,
    },
    /// The lobby: how many people are deciding, and whether the swiping has begun.
    ///
    /// `deciders` is the roster count — the number a recipe has to win over. It comes
    /// from who *joined the plan*, not from who has voted or who happens to be
    /// connected: a person who steps away is still deciding, and a person who has not
    /// swiped yet has not agreed to anything.
    /// The lobby: how many people are deciding, whether the swiping has begun — **and
    /// the two facts everything a room derives is derived from** (#212).
    ///
    /// `seed` and `created_at` ride here rather than on a frame of their own because
    /// this is the frame every client already receives on connect and on every change,
    /// and because they are facts *about the plan* exactly as its roster size and its
    /// startedness are. They are immutable for the life of the plan, so re-stating them
    /// costs two numbers and means a client can never be holding one without the other.
    ///
    /// `seed` is `None` for a plan created before plans had one (migration 0031), which
    /// has no shared randomness — the honest absence, not a zero.
    Lobby {
        deciders: i64,
        started: bool,
        /// The number this plan's shared randomness dangles off.
        seed: Option<i64>,
        /// When the plan was born, unix seconds — the anchor it is measured from.
        created_at: i64,
    },
    /// One live vote — drives both the incremental tally and peer-injection (a
    /// client slips `source`/`id` into its own deck if it has not seen it).
    Vote {
        voter: String,
        source: String,
        id: String,
        vote: bool,
    },
    /// The buy checklist for one recipe changed (#131) — the **whole** current
    /// list, not the one item that moved.
    ///
    /// Sending the list rather than a delta is what makes a shared checklist
    /// self-healing: a client that missed a frame (the broadcast ring lagged, or
    /// it was mid-reconnect) is corrected by the next one instead of drifting
    /// further, and there is no ordering to get wrong when two people tick at
    /// once. The list is one recipe's ingredients, so it is small enough that
    /// re-sending it is cheaper than the reconciliation a delta would need.
    Buy {
        source: String,
        id: String,
        checks: Vec<BuyCheck>,
    },
    /// Someone left the plan (#96): who, and whether that emptied it.
    ///
    /// Always from a lobby — the roster closes at the start in both directions —
    /// so the [`ServerMsg::Lobby`] frame beside it already carries the smaller
    /// roster. This is the event behind it, named rather than merely counted.
    Left {
        /// Who went. The whole person, like [`BuyCheck::by`] — the room says a
        /// name out loud, and by the time this arrives the roster no longer holds
        /// them to look one up from.
        voter: Voter,
        /// Whether they were the last, so the plan itself is gone. This is the
        /// half no other frame can carry: with the roster empty there is no
        /// smaller lobby to announce, and anyone still holding the channel — the
        /// leaver's own second tab, someone watching a lobby they were never
        /// seated into — would otherwise sit on a plan that no longer exists.
        ended: bool,
    },
    /// The plan decided (#201): every decider said yes to this recipe, nobody said
    /// no, and the server has **recorded** it on `pick_sessions`.
    ///
    /// The one frame that ends a pick. It is not an announcement of something the
    /// clients already worked out — since #201 they do not work it out — so it is
    /// sent twice over, and both sends are the same row read the same way:
    ///
    /// - to the room the instant the deciding vote lands, so everyone moves together;
    /// - to every socket **on connect** (see [`socket_loop`]), which is the durability
    ///   half of #201. A member whose browser was closed when the last yes arrived
    ///   opens the plan days later and is *told* what was decided, rather than
    ///   re-deriving it from a tally that merely still happens to satisfy the
    ///   condition.
    ///
    /// It goes to the **room**, not to the roster, so a watcher (#180) learns it the
    /// way everyone else does — nobody is left holding a plan whose deck is over.
    Decided {
        source: String,
        id: String,
        /// When it was recorded, from the database's clock — the same column the
        /// `WHERE decided_at IS NULL` guard is written against, so what a client is
        /// shown is the write that *made* this the decision.
        decided_at: i64,
    },
    /// **The server's clock, asking for yours** — the drift measurement the event
    /// framework runs on ([`crate::events::ClockOffset`]).
    ///
    /// Sent on connect and then on every keepalive tick (~30s), so the offset recorded
    /// for a connection tracks a clock that wanders over a plan's days rather than being
    /// measured once at the start. A client answers with [`ClientMsg::TimePong`],
    /// echoing `server_ms` and adding its own reading; nothing else about a client's
    /// clock is ever asked for.
    TimePing { server_ms: i64 },
    /// **What the server has measured this connection's clock to be doing**: how far
    /// ahead of the shared timeline it reads, and the round trip that estimate came
    /// from.
    ///
    /// Sent back so both sides work off *one* number. The client stamps its events with
    /// its own clock and the server subtracts this to normalise them; the client adds it
    /// back to render shared instants. The initiator's own countdown and everyone
    /// else's therefore agree exactly, rather than each side estimating separately and
    /// disagreeing by the difference.
    ///
    /// `rtt_ms` is the estimate's error bar — half of it bounds the offset's error — so
    /// a surface can say how well it knows the shared timeline instead of implying it
    /// knows it perfectly.
    TimeSync { offset_ms: i64, rtt_ms: i64 },
    /// **The plan's shared cook timers for one recipe** (#208), whole.
    ///
    /// Whole rather than a delta, for [`ServerMsg::Buy`]'s reason: a client that missed
    /// a frame is corrected by the next one instead of drifting, and two people tapping
    /// at once leave no ordering to get wrong. Sent to every socket on connect (the
    /// rehydrate half — a pot does not stop because a browser closed) and to the room on
    /// every start and dismiss.
    ///
    /// Instants are in the **shared timeline**; a client translates them through the
    /// offset it was given above. Finished timers are in the list — dismissing is what
    /// removes one, not time passing.
    Timers {
        source: String,
        id: String,
        timers: Vec<crate::timers::RunningTimer>,
    },
    /// **The plan is cooking** (#211): somebody in it tapped "Let's cook!", and the
    /// server has recorded that against the plan.
    ///
    /// The frame that moves the room to the stove, and the exact counterpart of
    /// [`ServerMsg::Decided`] one step later in the arc. It is sent twice over, and both
    /// sends are the same two columns read the same way:
    ///
    /// - to the room the instant the tap lands, so everybody goes together — including
    ///   the person who tapped, who navigates on the announcement like everybody else
    ///   rather than on their own click. One path, so the initiator cannot arrive
    ///   somewhere the room did not;
    /// - to every socket **on connect**, which is the durability half (#202): a member
    ///   who dropped, or who came back into the plan through their kitchen (#207), is
    ///   *told* the cook is on rather than left holding a shopping list nobody is
    ///   shopping from.
    ///
    /// It goes to the **room**, not to the roster, so a watcher (#180/#200) comes along
    /// exactly as they were carried through the decision — read-only, since every write
    /// they could make from the stove is refused at the framework's guard.
    ///
    /// Carries no recipe. A plan cooks what it decided, and that is already a server
    /// fact the client holds; naming it again here would be a second answer to what the
    /// plan is having.
    Cooking {
        /// When the cook started, in the **shared timeline** (unix ms) — the initiator's
        /// own tap, corrected for their measured clock drift by
        /// [`crate::events::normalize`].
        started_at: i64,
        /// Whose tap it was. The whole person, like [`BuyCheck::by`] and
        /// [`crate::timers::RunningTimer::started_by`] — a room that is told the cook
        /// started should not have to join back against the roster to say by whom.
        started_by: Voter,
    },
}

/// `pub(crate)` only because [`ServerMsg`] is: the event framework builds frames, so the
/// wire enum reaches outside this module and everything it names has to reach as far.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TallyRow {
    source: String,
    id: String,
    yes: i64,
    no: i64,
    /// Who said yes, by telegram id — the attribution behind the count.
    ///
    /// The live [`ServerMsg::Vote`] frame has always named its voter, but a
    /// client that (re)connects rehydrates from the tally, so a count alone would
    /// mean attribution survived only as long as the socket did. Turso is the
    /// truth here as everywhere: the tally carries who, not just how many.
    yes_voters: Vec<String>,
}

// ---- meal type -------------------------------------------------------------

/// Which meal a plan is for (#114): the strongest filter there is on what belongs
/// in the deck — nobody swipes pancakes and a lamb roast in the same session.
///
/// **The same type the corpus is read into** ([`recipe_core::meal::Sitting`], #191), not
/// a lookalike beside it. That reading answers "when is this dish eaten" with a set of
/// these words, and the walk's bound is then literally `sittings.contains(&meal)` — no
/// mapping, and so no second chance to disagree about what "dinner" means. The
/// vocabulary used to say a coming meal-type reading "can share the same words"; sharing
/// the type is how it does.
///
/// The full vocabulary is two tiers, and the tier is the type. This one is the
/// **primary** tier — the meals you sit down to: breakfast, lunch, dinner, a
/// snack. The **secondary** tier ([`MealAddition`]: dessert, side) is the things
/// that come *with* a meal. Splitting them into two types is what makes
/// the invalid states unrepresentable: a plan's meal type simply cannot be
/// "dessert" (it would claim the whole session for something that accompanies
/// it), and a chosen addition cannot be "dinner" — serde refuses both at the
/// wire, no handler checks anything.
///
/// A **fixed vocabulary**, not free text: unlike ingredients this is a small
/// closed set, so a picker over it can be exhaustive and stable. Serde owns the wire
/// form — always the lowercase name, and an unknown or wrongly-cased value is rejected
/// at deserialization, so no handler ever holds a word outside its tier. The browser
/// sentence-cases for display; the wire and the database stay lowercase.
pub type MealType = recipe_core::meal::Sitting;

/// A plan that names no meal is for dinner — the meal a group most plausibly
/// plans together. The same word migration 0016 backfills, so an unstated choice
/// and a pre-migration row read identically. Not time-of-day inference: it is one
/// fixed word, and the host changes it in the lobby if it is wrong.
///
/// A constant here rather than a `Default` on the type, because it is a decision about
/// **a plan** and not about the vocabulary: a *dish* has no default sitting, and giving
/// the shared type one would let a reading quietly default to dinner instead of being
/// refused as empty (#191).
pub const DEFAULT_MEAL_TYPE: MealType = MealType::Dinner;

/// A secondary choice on a plan (#114): something that comes *with* the meal — a
/// dessert, a side — never the meal itself. See [`MealType`] for the two-tier
/// split; this is the tier a plan can carry **several** of, alongside exactly one
/// meal.
///
/// **Every word here is one the corpus states about a recipe**: 166 recipes are
/// a `Dessert` and 84 are a `Side`. `Drink` was a third variant and is not one
/// any more (#185) — no source we ingest carries drinks (0 categories, 0 tags
/// across 790 recipes), so a host could toggle it and there was nothing in the
/// world for it to mean. That is not a claim that a drink is not part of a meal;
/// it is that nothing we read supplies one. A drinks adapter puts the variant
/// back, with data behind it.
///
/// Chosen additions are recorded on the session and shown in the lobby, so the
/// room knows dinner comes with dessert. Whether a chosen addition one day gets
/// its **own pick round** (swipe the dinner, then swipe the dessert) is a real
/// possibility and deliberately not built here — this slice records and shows
/// the choice; the pick still runs one round, for the meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MealAddition {
    Dessert,
    Side,
}

impl MealAddition {
    /// Every addition, in canonical order — the order stored, and the order the
    /// picker shows.
    pub const ALL: [MealAddition; 2] = [MealAddition::Dessert, MealAddition::Side];
}

/// The canonical form of a chosen-additions list: each addition at most once, in
/// vocabulary order. The list means a *set* — "dessert and a side" — so a double
/// tap or a reordered client must not mint a different plan.
fn normalize_additions(input: &[MealAddition]) -> Vec<MealAddition> {
    MealAddition::ALL
        .iter()
        .copied()
        .filter(|a| input.contains(a))
        .collect()
}

// ---- HTTP handlers ---------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    /// Optional JSON scope that seeds each participant's feed. Opaque here — the
    /// client interprets it; the backend only stores and echoes it.
    #[serde(default)]
    filter: Option<String>,
    /// The kitchen this plans a meal for. Optional so a plan can still be started
    /// outside one.
    #[serde(default)]
    kitchen_id: Option<String>,
    /// Which meal this plans (#114). Optional — an unstated choice is dinner
    /// ([`DEFAULT_MEAL_TYPE`]) and the host can change it in the lobby.
    #[serde(default)]
    meal_type: Option<MealType>,
    /// What comes with it (#114) — a dessert, a side. Optional; none is a plain
    /// meal, and the host can add them in the lobby.
    #[serde(default)]
    additions: Vec<MealAddition>,
    /// The plan's total-time cap in seconds (#80); `null` = no cap ("Any"). Not
    /// opaque like `filter`: the walk enforces it server-side against
    /// `recipes.total_seconds`, so the backend must understand it.
    ///
    /// **Absent and `null` are different here** (#163). A body that says nothing
    /// gets [`DEFAULT_CAP_SECONDS`] — a plan is born capped. A body that says
    /// `null` gets "Any", the same word that lifts the cap in the lobby. That is
    /// what keeps the default a default rather than a floor: the caller who wants
    /// the whole corpus asks for it, instead of being unable to say so.
    #[serde(default = "default_cap")]
    max_total_seconds: Option<i64>,
}

/// The bounds a time cap must sit in: at least a minute, at most a day.
///
/// The UI presents fixed buckets (30 min / 1 hour / 2 hours / Any); the API
/// deliberately accepts any sane number of seconds instead of that enum, so the
/// buckets stay a presentation choice rather than a schema — changing them is a
/// frontend edit, not a migration (#80).
const MIN_CAP_SECONDS: i64 = 60;
const MAX_CAP_SECONDS: i64 = 86_400;

/// What a plan is born capped at (#163): half an hour.
///
/// A plan used to be born unbounded, which put the control on the one setting that
/// filters nothing — inert until somebody touched it — and cheerfully offered a
/// five-hour braise to whoever is hungry now. Half an hour is where most meals
/// live, and the person with the afternoon widens it in one tap.
///
/// It bounds the deck to what we can *prove* fits plus what we cannot time at all:
/// `recipes.total_seconds` counts untimed steps as zero (#84, #158) and a recipe
/// with no estimate is deliberately kept under a cap, so the exclusions are sound
/// (a lower bound already over the cap really is over it) while the inclusions are
/// optimistic. Measured against the corpus when this landed, 1800 seconds left
/// 390 of 790 recipes — 313 estimated at or under it, plus 77 with no estimate.
///
/// The column carries the same number as its default (migration 0019), so a row
/// inserted without one and a plan created without one read identically — the same
/// pairing #114's `meal_type` makes with `'dinner'`.
const DEFAULT_CAP_SECONDS: i64 = 1800;

/// Serde's stand-in for an absent `max_total_seconds`. Only reached when the field
/// is missing entirely; an explicit `null` deserializes to `None` ("Any") and never
/// consults this.
fn default_cap() -> Option<i64> {
    Some(DEFAULT_CAP_SECONDS)
}

/// Refuse a nonsense cap. `None` is "any" and always fine; zero, negative, and
/// longer-than-a-day are author errors, not bounds anyone cooks to.
fn validate_cap(cap: Option<i64>) -> Result<(), AppError> {
    match cap {
        None => Ok(()),
        Some(s) if (MIN_CAP_SECONDS..=MAX_CAP_SECONDS).contains(&s) => Ok(()),
        Some(s) => Err(AppError::BadRequest(format!(
            "max_total_seconds must be between {MIN_CAP_SECONDS} and {MAX_CAP_SECONDS}, got {s}"
        ))),
    }
}

#[derive(Debug, Serialize)]
pub struct Created {
    channel_id: String,
}

/// `POST /api/session` — start a session, returning its shareable channel id.
pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<CreateBody>,
) -> Result<Json<Created>, AppError> {
    validate_cap(body.max_total_seconds)?;
    // Minted once, *outside* the retryable closure, so every attempt writes the
    // same plan: a re-run after a lost response then hits the primary key and
    // fails honestly instead of minting a second plan (#130).
    let channel_id = mint_channel_id();
    // One closure — one connection per attempt — for both writes: the plan and
    // its first voter are one act, and two connections to a local database can
    // contend for the same write lock.
    {
        let channel_id = &channel_id;
        let user = &user;
        let body = &body;
        state.with_db(move |db| async move {
            create_session(
                &db,
                channel_id,
                &user.telegram_user_id,
                body.filter.as_deref(),
                body.kitchen_id.as_deref(),
                body.meal_type.unwrap_or(DEFAULT_MEAL_TYPE),
                &body.additions,
                body.max_total_seconds,
            )
            .await?;
            // The host is in their own plan from the moment it exists, so a lobby is
            // never empty and a plan never has nobody deciding it.
            seat_voter(&db, channel_id, &user.telegram_user_id).await
        })
    }
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(Created { channel_id }))
}

/// Who the host is pulling into the plan — a member of its kitchen, by their id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SeatBody {
    pub user_id: String,
}

/// A person in a plan. `username` is display convenience; identity is the id (#25).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Voter {
    pub telegram_user_id: String,
    pub username: Option<String>,
}

/// One ticked line of a meal's shopping list (#131): which ingredient, and where the
/// tick came from.
///
/// **Exactly one of `by` and `pantry` is set** — the database says so with a CHECK
/// (migration 0021), and the two are genuinely different claims:
///
/// - `by` — a person got it. The whole person rather than an id, because every surface
///   that shows a tick shows *who*; a bare id would make the browser join it back
///   against the roster, and a shopper who never joined the lobby (there is no such
///   thing today, but the roster is not this table's business) would render as a blank.
/// - `pantry` — the plan's kitchen already had it (#156), and this is the pantry entry
///   that answered for the line. Nobody claimed it, so nobody's colour goes on it: a
///   colour means a person, and the cupboard is not one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuyCheck {
    /// The 0-based position in the recipe's ingredient list.
    pub index: i64,
    /// Who got it, when a person did.
    pub by: Option<Voter>,
    /// The pantry entry that pre-ticked it, when the kitchen already had it.
    pub pantry: Option<String>,
}

/// What a plan decided (#201): the one recipe its roster agreed on, and when the
/// server recorded that.
///
/// A **record**, not a computation. The win condition is evaluated inside the write
/// that stores this (see [`decide_if_agreed`]), so the presence of this value is the
/// decision — nothing downstream re-checks the tally, and nothing can recompute the
/// answer away if the tally later changes. That is the property #201 exists for: a
/// plan runs for days, and "what we decided" has to be somewhere rather than being
/// re-derived by whichever browser happens to be open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecidedRecipe {
    pub source: String,
    pub id: String,
    /// Unix seconds, from the database's clock. The column the first-past-the-post
    /// guard is written against, so it says *whether* as much as *when*.
    pub decided_at: i64,
}

/// A meal's shopping checklist for one recipe (#131) — every line already got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuyList {
    pub channel_id: String,
    pub source: String,
    pub id: String,
    /// The ticked lines, in ingredient order. An unticked line simply is not here.
    pub checks: Vec<BuyCheck>,
}

/// A plan's lobby: who is deciding, and whether it has begun.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LobbyView {
    pub channel_id: String,
    pub kitchen_id: Option<String>,
    /// Which meal this plans (#114) — what the room is deciding, so every voter
    /// sees it, and what the deck will one day be filtered by.
    pub meal_type: MealType,
    /// What comes with it (#114) — the chosen secondary things, each at most
    /// once, in vocabulary order. Recorded and shown; the pick itself still runs
    /// one round, for the meal.
    pub additions: Vec<MealAddition>,
    /// The telegram id that started it — only they can start the swiping.
    pub host: String,
    pub started: bool,
    /// The plan's total-time cap in seconds (#80); `None` = no cap. Everyone in the
    /// lobby sees the bound they will be swiping within.
    pub max_total_seconds: Option<i64>,
    /// Whether we know what this plan's kitchen owns (#82) — i.e. whether it has any
    /// equipment recorded at all.
    ///
    pub voters: Vec<Voter>,
    /// Members of the plan's kitchen who are not yet deciding — the host can seat any
    /// of them without a link (#72). Empty when the plan has no kitchen, or once
    /// everyone in the kitchen is already in.
    pub candidates: Vec<Voter>,
    /// What this plan decided (#201), or `None` while its deck is still running.
    ///
    /// Carried here as well as on [`ServerMsg::Decided`] because a lobby read that
    /// cannot say a plan is over is a silent state: this is the one HTTP answer that
    /// describes the whole plan, and it is what the page has already read on mount
    /// before its socket has finished rehydrating. Both are the same three columns of
    /// the same row, so there is one answer to "what did we pick", not two.
    pub decided: Option<DecidedRecipe>,
    /// **This plan's seed** (#212) — the number its shared randomness dangles off, or
    /// `None` for a plan created before plans had one, which has no shared randomness
    /// and honestly says so (migration 0031).
    pub seed: Option<i64>,
    /// **When the plan was born**, unix seconds — the anchor everything derived from the
    /// seed is measured from. Whole seconds because every device reads *this* stored
    /// value, so the coarseness is shared and disagrees with nobody.
    pub created_at: i64,
}

/// `GET /api/session/{channel}` — the lobby: the roster, and whether it has started.
pub async fn lobby(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))
        .map(Json)
}

/// `POST /api/session/{channel}/join` — join a plan as a decider.
///
/// Only while the lobby is open. Once the swiping has begun the roster is what the
/// tally is measured against, so admitting someone late would move the target for
/// everyone already voting — every recipe that had won unanimously would silently
/// stop having done so.
pub async fn join_lobby(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let user = &user;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    if view.started
        && !view
            .voters
            .iter()
            .any(|v| v.telegram_user_id == user.telegram_user_id)
    {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    state
        .with_db(move |db| async move { seat_voter(&db, channel, &user.telegram_user_id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// What a departure left behind (#96) — enough for the leaver's own screen to know
/// where it stands, without a second read of a plan they are no longer in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Departure {
    pub channel_id: String,
    /// The kitchen the plan was for, so leaving puts you back where you came from.
    /// `None` for a plan started outside one — the client falls back to your own.
    pub kitchen_id: Option<String>,
    /// Whether that was the last person, so the plan is gone rather than smaller.
    pub plan_ended: bool,
    /// Who holds the plan now; `None` when it ended. Differs from the caller
    /// exactly when the host left and it passed on.
    pub host: Option<String>,
}

/// `DELETE /api/session/{channel}/join` — leave a meal plan (#96).
///
/// The inverse of [`join_lobby`], and deliberately the same path: joining and
/// leaving are one thing said in two directions, so they are one resource with two
/// verbs rather than a `/join` and a `/leave` that could drift apart.
///
/// **Leaving is a lobby act, like joining.** The roster closes at the start in
/// *both* directions (#93): the people swiping agreed to decide together, and the
/// number they have to agree on may no more fall out from under them than a late
/// joiner may raise it. A departure landing after the start is refused with the same
/// 400 every other lobby write gives, and the guard lives in the delete's own
/// predicate (see [`remove_voter`]) rather than in a preceding read, so a start
/// arriving between the two wins.
///
/// So nothing but the roster moves. There are no votes to retract and no shopping
/// claims to release, because neither exists before the swiping begins — the room is
/// told the smaller roster and then who left it (see [`ServerMsg::Left`]), in that
/// order, so a client reading them in sequence never renders "someone left" against
/// a roster that still contains them.
///
/// Guards in the house style: an unknown channel is a client bug (400), and someone
/// who was never in the plan is refused (403) rather than quietly answered — the same
/// answer every roster-gated HTTP write in this module gives a stranger holding the
/// channel id.
pub async fn leave_lobby(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<Departure>, AppError> {
    let channel = channel.as_str();
    let user = &user;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    let Some(who) = view
        .voters
        .iter()
        .find(|v| v.telegram_user_id == user.telegram_user_id)
        .cloned()
    else {
        return Err(AppError::Forbidden("you are not in this meal plan".into()));
    };

    let departed = state
        .with_db(move |db| async move { remove_voter(&db, channel, &user.telegram_user_id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let (plan_ended, host) = match departed {
        Removed::Left { host } => (false, Some(host)),
        Removed::Ended => (true, None),
        // The roster closed while this was in flight. The same answer every other
        // lobby write gives, because it is the same fact.
        Removed::Started => {
            return Err(AppError::BadRequest(
                "this meal plan has already started".into(),
            ));
        }
    };

    // Tell the room before answering the leaver, so the people still in the plan and
    // the person walking out of it learn the same thing from the same act. The roster
    // frame goes first and the event that explains it last: a client that reads them in
    // order never renders "someone left" against a roster that still contains them.
    //
    // Only the roster moves. Leaving is a lobby act, so there are no votes to retract
    // and no shopping claims to release — nothing else about the meal is decided yet.
    if !plan_ended {
        reload_and_announce(&state, channel).await?;
    }
    announce_departure(&state, channel, who, plan_ended);

    Ok(Json(Departure {
        channel_id: channel.to_owned(),
        kitchen_id: view.kitchen_id,
        plan_ended,
        host,
    }))
}

/// `POST /api/session/{channel}/start` — close the lobby and begin the pick. Host only.
pub async fn start(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can begin it".into(),
        ));
    }
    state
        .with_db(move |db| async move { begin_session(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// `POST /api/session/{channel}/seat` — the host pulls a kitchen member into the plan.
///
/// The counterpart to the invite link: the link is for people outside the kitchen, this
/// is for people already in it, who should not have to be sent anything. A meal is
/// planned *in* a kitchen (#72), so the seatable pool is exactly that kitchen's
/// members — seating an arbitrary id is refused.
///
/// Host only, and only before the swiping starts: the roster is what consensus is
/// measured against (#93), so it must not grow once people are voting.
///
/// A seated member is a decider whether or not they have opened the app yet — the
/// plan then waits on them, which is the point ("we're waiting on Mel"). If they are
/// not in fact cooking tonight, the host does not seat them, or they leave (#96).
pub async fn seat(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<SeatBody>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let body = &body;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can add people to it".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    let Some(kitchen_id) = view.kitchen_id.as_deref() else {
        return Err(AppError::BadRequest(
            "this plan is not in a kitchen, so it has no members to add — share the link".into(),
        ));
    };
    if !state
        .with_db(move |db| async move {
            crate::kitchens::is_member(&db, kitchen_id, &body.user_id).await
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        return Err(AppError::BadRequest(
            "that person is not in this kitchen — share the link instead".into(),
        ));
    }

    state
        .with_db(move |db| async move { seat_voter(&db, channel, &body.user_id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// The meal the host is declaring the plan to be for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct MealTypeBody {
    pub meal_type: MealType,
}

/// `POST /api/session/{channel}/meal-type` — the host names which meal this plans.
///
/// Every plan is born for dinner (the create default), so this is how the lobby
/// "picks one" (#114): the host flicks it to breakfast, lunch, a snack — and the
/// room announcement re-reads the lobby on every open client, so the whole roster
/// sees what it is deciding.
///
/// Host only, and only before the swiping starts — same shape as [`seat`], for the
/// same reason: once people are voting, the terms of the plan must not move under
/// them. A dinner nobody agreed on cannot retroactively become an agreed breakfast.
pub async fn set_meal_type(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<MealTypeBody>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can change which meal it is for".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }

    // The write carries the not-started condition too (see `set_time_cap`).
    let written = state
        .with_db(move |db| async move { update_meal_type(&db, channel, body.meal_type).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !written {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// The secondary things the host is declaring alongside the meal. The whole
/// chosen set each time — a set, not a delta — so the picker's state and the
/// stored state cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AdditionsBody {
    pub additions: Vec<MealAddition>,
}

/// `POST /api/session/{channel}/additions` — the host names what comes with the
/// meal: a dessert, a side.
///
/// Same guards as [`set_meal_type`], for the same reason — host only, and only
/// while the lobby is open; once people are voting, the terms of the plan must
/// not move under them. Announced to the room so every open client re-reads the
/// lobby.
pub async fn set_additions(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<AdditionsBody>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let body = &body;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can change what comes with the meal".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }

    // The write carries the not-started condition too (see `set_time_cap`).
    let written = state
        .with_db(move |db| async move { update_additions(&db, channel, &body.additions).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !written {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// What the host is bounding the plan to — seconds, or `null` to lift the cap.
#[derive(Debug, Deserialize)]
pub struct CapBody {
    #[serde(default)]
    max_total_seconds: Option<i64>,
}

/// `POST /api/session/{channel}/cap` — the host sets (or lifts) the plan's time cap.
///
/// Same guards as [`set_meal_type`], for the same reason — host only, and only
/// while the lobby is open: the cap defines the shared corpus everyone in the
/// session swipes within (#80), so it must not move once people are voting. Until
/// then the host is still deciding what tonight is (the session itself is minted
/// the moment the pick page opens, before any choice could be made), so the lobby
/// is where the choice lands. Announced to the room so every open client sees the
/// new bound at once.
pub async fn set_cap(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<CapBody>,
) -> Result<Json<LobbyView>, AppError> {
    validate_cap(body.max_total_seconds)?;
    let channel = channel.as_str();
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can set its time cap".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }

    // The write carries the not-started condition too, so a start() that landed since
    // the read above wins and this changes nothing (see `set_time_cap`).
    let written = state
        .with_db(move |db| async move { set_time_cap(&db, channel, body.max_total_seconds).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !written {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

// ---- buy checklist (#131) --------------------------------------------------

/// Which recipe's checklist is being asked about — the plan's decided recipe.
///
/// **It is no longer taken on trust (#201).** The recipe still travels in the query,
/// because the checklist is keyed by it and a re-decided plan must not inherit a stale
/// list — but the plan now *holds* its decision ([`DecidedRecipe`]), so what a client
/// names is checked against it and refused when the two disagree
/// ([`decided_recipe_or_refuse`]). Before, this was the whole story: any seated
/// member's client could name any `(source, id)` and the server obliged — built the
/// list, seeded the pantry, took the ticks — so two clients could shop two different
/// dinners on one channel and nothing had an opinion about which was the meal.
///
/// A plan with **no** recorded decision still admits any recipe, which is the only
/// honest reading of a column that was not backfilled: migration 0026 could not
/// reconstruct decisions that were only ever made in browsers, so a list stashed before
/// this deployed keeps working rather than being told it never happened.
#[derive(Debug, Deserialize)]
pub struct BuyQuery {
    pub source: String,
    pub id: String,
}

/// Refuse a shopping request that names a recipe this plan decided against (#201).
///
/// The **honest 4xx**: it names what the plan actually decided, because the caller's
/// next question is always "then what did we pick", and a bare "no" would send someone
/// hunting a fault in their own client. A client bug rather than a permissions problem,
/// so 400 and not 403 — the person is allowed here, the recipe is not.
///
/// Read *and* said here so the refusal has a sentence; repeated inside the writes'
/// own predicates ([`not_against_the_decision`]) so it is also true. Since #209 the
/// only caller left is [`buy_list`], the read — a shopping *write* is an event now, and
/// events are refused in silence at the framework's choke point (#179/#180), so the
/// predicate inside the write is the whole of what keeps that row out.
async fn decided_recipe_or_refuse(
    state: &AppState,
    channel: &str,
    source: &str,
    id: &str,
) -> Result<(), AppError> {
    let decided = state
        .with_db(move |db| async move { load_decision(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    match decided {
        Some(d) if d.source != source || d.id != id => Err(AppError::BadRequest(format!(
            "this meal plan decided on {}/{}, not {source}/{id}",
            d.source, d.id
        ))),
        _ => Ok(()),
    }
}

/// `GET /api/session/{channel}/buy?source=…&id=…` — the meal's shopping checklist.
///
/// Readable by any signed-in caller holding the channel id, exactly like the lobby
/// ([`lobby`]) whose roster it names: the two answer the same question about the
/// same meal, so gating one and not the other would only mean a person who can see
/// that Mel is deciding cannot see that Mel got the carrots.
///
/// This is also where a list is **built**, so it is where the pantry seed happens
/// (#156) — see [`ensure_buy_seed`] for why a read is the right moment and why it is
/// safe here despite not being roster-gated. #170's reasoning survives #201 intact and
/// gets stronger: the seed still depends on nothing about the caller, and the recipe it
/// runs for is now a fact the plan holds rather than a string the caller supplied.
///
/// **The recipe is checked before anything is built** ([`decided_recipe_or_refuse`]).
/// A list for a recipe the plan decided against is not a list anyone should be handed,
/// and building one would also read a kitchen's pantry against a meal nobody agreed to
/// cook.
pub async fn buy_list(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Query(q): Query<BuyQuery>,
) -> Result<Json<BuyList>, AppError> {
    let channel = channel.as_str();
    let q = &q;
    if !state
        .with_db(move |db| async move { session_exists(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        return Err(AppError::BadRequest(format!("unknown session: {channel}")));
    }
    decided_recipe_or_refuse(&state, channel, &q.source, &q.id).await?;
    ensure_buy_seed(&state, channel, &q.source, &q.id).await?;
    let checks = state
        .with_db(move |db| async move { load_buy_checks(&db, channel, &q.source, &q.id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(BuyList {
        channel_id: channel.to_owned(),
        source: q.source.clone(),
        id: q.id.clone(),
        checks,
    }))
}

/// Re-read the lobby and tell the room, so every open client moves together — a guest
/// arriving, or the host pressing start, lands on everyone's screen at once.
async fn reload_and_announce(state: &AppState, channel: &str) -> Result<LobbyView, AppError> {
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    let tx = room(&state.rooms, channel);
    if let Ok(txt) = serde_json::to_string(&ServerMsg::Lobby {
        deciders: view.voters.len() as i64,
        started: view.started,
        seed: view.seed,
        created_at: view.created_at,
    }) {
        // No receivers is an error and also a non-event: nobody is listening yet.
        let _ = tx.send(txt);
    }
    Ok(view)
}

/// Name whoever just left to the room (#96) — the event behind the roster frame that
/// went before it. Infallible on purpose: the durable state is already
/// written, so a room nobody is listening to must not turn a completed departure
/// into an error.
fn announce_departure(state: &AppState, channel: &str, voter: Voter, ended: bool) {
    let tx = room(&state.rooms, channel);
    if let Ok(txt) = serde_json::to_string(&ServerMsg::Left { voter, ended }) {
        let _ = tx.send(txt);
    }
}

/// `GET /api/session/{channel}/ws` — join a session's live room.
///
/// Session-gated like every person-facing route (#25); the upgrade carries the
/// session cookie, so the socket knows who is voting.
///
/// **Anyone signed in with the channel id may listen, and only a decider may write.**
/// The upgrade deliberately asks nothing about the roster — someone who followed the
/// link after the swiping began cannot join (`join_lobby` refuses them) but can still
/// watch it happen, and this socket is how watching works (#180/#200); they can already
/// read the same lobby, tally and shopping list over HTTP. What must not follow from
/// holding the link is a *write*, so the refusal lives on the event, at the framework's
/// guard ([`crate::events::ingest`]) and again inside each handler's own predicate,
/// rather than on the door (#175).
///
/// Since #209 that is the whole of the write surface for a plan mid-flight: votes and
/// shopping ticks came here from their own paths, so a watcher is refused by one rule in
/// one place instead of by three rules in three.
pub async fn ws(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    // An unknown channel is a client bug, not a new room to conjure.
    if !{
        let channel = channel.as_str();
        state.with_db(move |db| async move { session_exists(&db, channel).await })
    }
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    {
        return Err(AppError::BadRequest(format!("unknown session: {channel}")));
    }
    let tx = room(&state.rooms, &channel);
    Ok(upgrade
        .on_upgrade(move |socket| socket_loop(socket, state, user.telegram_user_id, channel, tx)))
}

/// One connected client: rehydrate, then fan votes both ways until it drops.
async fn socket_loop(
    socket: WebSocket,
    state: AppState,
    voter: String,
    channel: String,
    tx: broadcast::Sender<String>,
) {
    let mut rx = tx.subscribe();
    let (mut sink, mut stream) = socket.split();

    // Not a handler: there is nowhere to return an error to, and this outlives a
    // request by design. It takes one connection for the life of the socket — if that
    // stream goes stale the client reconnects, which is the same recovery a dropped
    // socket already has.
    let Ok(db) = state.database.connect() else {
        return;
    };

    // This connection's recorded clock offset — the event framework's per-participant
    // drift measurement (`events::ClockOffset`). It lives exactly as long as the socket:
    // an offset describes *this* connection, and a reconnect re-measures in one round
    // trip rather than being served out of a record of how wrong a phone used to be.
    let mut clock = crate::events::ClockOffset::new();

    // First measurement before anything else, so a client that taps immediately is
    // already normalised through a real sample rather than the honest-but-blind zero.
    let ping = crate::events::server_now_ms();
    clock.ping_sent(ping);
    if let Ok(txt) = serde_json::to_string(&ServerMsg::TimePing { server_ms: ping }) {
        if sink.send(Message::Text(txt.into())).await.is_err() {
            return;
        }
    }

    // Rehydrate: the current tally before any live vote.
    if let Ok((participants, votes)) = load_tally(&db, &channel).await {
        if let Ok(txt) = serde_json::to_string(&ServerMsg::Tally {
            participants,
            votes,
        }) {
            if sink.send(Message::Text(txt.into())).await.is_err() {
                return;
            }
        }
    }

    // The lobby, so a (re)connecting client knows how many it has to convince and
    // whether the swiping has begun, without a second round trip.
    if let Ok(Some(view)) = load_lobby(&db, &channel).await {
        if let Ok(txt) = serde_json::to_string(&ServerMsg::Lobby {
            deciders: view.voters.len() as i64,
            started: view.started,
            seed: view.seed,
            created_at: view.created_at,
        }) {
            if sink.send(Message::Text(txt.into())).await.is_err() {
                return;
            }
        }

        // And what the plan decided, if it has (#201) — the durability half. This is
        // the frame a client is *told* the answer by, and it has to arrive here as
        // well as live, because the person it matters most to is the one whose browser
        // was closed when the last yes landed. They are not re-deriving it from the
        // tally above; they are being handed the record, which stands whatever that
        // tally has since become. It goes to whoever opened the socket, so a watcher
        // (#180) is told too.
        //
        // Last of the three on purpose: a client that reads its frames in order has
        // the roster and the votes in hand before it is told the deck is over, so the
        // screen it lands on is never a decision against an empty tally.
        if let Some(d) = view.decided {
            if let Ok(txt) = serde_json::to_string(&ServerMsg::Decided {
                source: d.source,
                id: d.id,
                decided_at: d.decided_at,
            }) {
                if sink.send(Message::Text(txt.into())).await.is_err() {
                    return;
                }
            }
        }
    }

    // And whether the room is already cooking (#211) — the same durability rule one step
    // further along the arc. A member who dropped, or who came back into the plan through
    // their kitchen (#207), is *told* the cook is on and lands at the stove, rather than
    // opening a shopping list nobody is shopping from. Sent to whoever opened the socket,
    // so a watcher (#200) comes along read-only exactly as they were carried through the
    // decision.
    //
    // After the decision on purpose: a client reads its frames in order, and being told
    // the room is cooking before being told what it decided would be a stove with no pot
    // named on it.
    if let Ok(Some(cook)) = load_cook(&db, &channel).await {
        if let Ok(txt) = serde_json::to_string(&ServerMsg::Cooking {
            started_at: cook.started_at,
            started_by: cook.started_by,
        }) {
            if sink.send(Message::Text(txt.into())).await.is_err() {
                return;
            }
        }
    }

    // And the pots that are on (#208). Plan state survives a drop and a long session
    // exactly the way the lobby and the decision above do: a countdown started an hour
    // ago is still counting, and one that finished while every browser was closed is
    // still a pot to take off the heat — so both come back here, and only a dismiss ever
    // removes one.
    //
    // Every recipe the plan has timers on, not merely the decided one, so a client is
    // never told about a timer it cannot see *or* left holding one it was never told
    // about; the cook screen ignores frames for a recipe it is not cooking, the way
    // `buy` ignores another recipe's checklist.
    if let Ok(all) = crate::timers::load_all(&db, &channel).await {
        for (source, id, timers) in all {
            if let Ok(txt) = serde_json::to_string(&ServerMsg::Timers { source, id, timers }) {
                if sink.send(Message::Text(txt.into())).await.is_err() {
                    return;
                }
            }
        }
    }

    // Render's free tier closes a WS idle for 5 min; a ping well inside that keeps
    // an active session's socket — and the box — awake. It is also the event
    // framework's drift-refresh cadence: the clock measurement rides this frame, so
    // tracking a wandering clock over a days-long plan costs no extra wake-ups.
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.tick().await; // the first tick fires immediately; consume it

    loop {
        tokio::select! {
            // A live vote from any peer (including this client's own echo) → forward.
            msg = rx.recv() => match msg {
                Ok(txt) => {
                    if sink.send(Message::Text(txt.into())).await.is_err() {
                        break;
                    }
                }
                // Fell behind the ring buffer; the client re-reads on its next
                // reconnect, so drop the gap rather than the connection.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            // A frame from this client.
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    // Every frame this client sends is read once, here, and routed —
                    // and since #209 there are only two places for it to go: the clock
                    // measurement into this connection's recorded offset, and *every*
                    // write through `events::ingest`. The vote had its own arm here
                    // until it migrated; nothing does now, which is what makes the
                    // choke point a choke point rather than a convention. An unreadable
                    // frame is dropped in silence — there is nowhere on this socket to
                    // answer one.
                    match serde_json::from_str::<ClientMsg>(&t) {
                    // The other half of one clock measurement (`events::ClockOffset`).
                    // The estimate goes straight back to the client that produced it, so
                    // both sides normalise through the *same* number rather than each
                    // estimating separately and disagreeing by the difference. A pong
                    // that answers no outstanding ping is dropped and says nothing.
                    Ok(ClientMsg::TimePong { server_ms, client_ms }) => {
                        if clock.pong(server_ms, client_ms, crate::events::server_now_ms()) {
                            if let Ok(txt) = serde_json::to_string(&ServerMsg::TimeSync {
                                offset_ms: clock.offset_ms(),
                                rtt_ms: clock.rtt_ms(),
                            }) {
                                if sink.send(Message::Text(txt.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    // **The event framework's one path in** (#208/#209) — a vote, a
                    // shopping tick, a timer, and whatever is added next. Normalising
                    // `at`, applying the kind's guard, writing and deciding what the
                    // room is told all live in `events::ingest`; this arm only carries
                    // what came back to the room.
                    //
                    // The rules are the ones the vote's own arm used to hold here:
                    // nothing is announced that was not written (an empty answer is a
                    // refusal, and refusals on this socket are silent — #179/#180), and
                    // a database fault leaves the plan as it was rather than announcing
                    // a write that never happened.
                    Ok(ClientMsg::Event { at, event }) => {
                        let frames = crate::events::ingest(
                            &db,
                            &channel,
                            &voter,
                            &clock,
                            at,
                            crate::events::server_now_ms(),
                            event,
                        )
                        .await
                        .unwrap_or_default();
                        for frame in frames {
                            if let Ok(txt) = serde_json::to_string(&frame) {
                                // Err only means no receivers right now — harmless.
                                let _ = tx.send(txt);
                            }
                        }
                    }
                    Err(_) => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                // Ping/pong are handled by axum; other frames are ignored.
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
            _ = keepalive.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
                // …and the drift refresh rides it. A plan runs for days or weeks (#202)
                // and clocks wander over that, so the offset recorded for this
                // connection is re-measured for as long as the connection lives rather
                // than being taken once at the start and trusted forever.
                let ping = crate::events::server_now_ms();
                clock.ping_sent(ping);
                if let Ok(txt) = serde_json::to_string(&ServerMsg::TimePing { server_ms: ping }) {
                    if sink.send(Message::Text(txt.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

// ---- persistence (pure, testable) ------------------------------------------

/// Seat someone in a plan. Idempotent — joining twice is one row, so a re-opened link
/// or a double tap does not inflate the number a recipe has to win over.
async fn seat_voter(conn: &Connection, channel: &str, user: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pick_voters (channel_id, user_id) VALUES (?1, ?2)
         ON CONFLICT(channel_id, user_id) DO NOTHING",
        libsql::params![channel, user],
    )
    .await?;
    Ok(())
}

/// What became of the plan when somebody left it.
///
/// Three outcomes rather than "who holds it now", because that cannot express the one
/// that matters most: a departure the plan **refused**. The roster is fixed once
/// swiping starts, so a leave arriving late changes nothing and has to say so rather
/// than reporting a plan with no host.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Removed {
    /// They are out and the plan carries on, now held by this telegram id.
    Left { host: String },
    /// They were the last one, so the plan went with them.
    Ended,
    /// Swiping had already begun, so the roster is closed and nothing was written.
    Started,
}

/// Take someone out of a plan's lobby (#96).
///
/// Idempotent from end to end — every statement is a delete or a conditional update,
/// so a retry after a lost response finishes the same departure rather than starting
/// a second one. Membership is judged by the caller's read, not by this function's
/// row counts, so two taps of the same button both succeed instead of the loser
/// being told it was never in the plan.
///
/// **Only from the lobby.** The seat delete carries `started_at IS NULL` in its own
/// predicate, so the roster is closed in both directions the instant the swiping
/// begins and a start landing mid-request wins rather than losing to a read taken a
/// moment earlier. A delete that writes nothing is then classified rather than
/// assumed: [`Removed::Started`] only when the plan really has started, because the
/// other reason to write nothing is a seat that was already gone — the same
/// departure arriving twice.
///
/// **Nothing else is theirs to take.** A vote and a shopping claim are each written
/// under `started_at IS NOT NULL` in the write's *own* predicate
/// ([`seated_in_a_started_plan`]), and the predicate above is the proof this plan had
/// not started — so there is deliberately no vote sweep and no claim sweep here.
/// Cleanup written for a state the guard makes unreachable is an invitation to relax
/// the guard later, believing it is handled. The one row that can exist on a plan
/// still in its lobby is a pantry pre-tick, and that is nobody's claim (#156): the
/// cupboard is not a person, so a leaver has nothing in it to release (#175).
///
/// **The host passes it on rather than taking the plan down.** Forbidding the host
/// to leave traps the one person who cannot escape their own plan — the hostage
/// problem again, pointed at them. Ending it means one tap destroys everyone else's
/// plan and roster, and a meal is not the host's to cancel once other people are
/// gathering for it. Leaving it hostless leaves a lobby nobody can
/// start. So it passes to the longest-standing remaining decider — the same order
/// the lobby lists people in, so "the next person in the room" is what everybody
/// already sees. Chosen *inside* the UPDATE, and only while the leaver still holds
/// it, so there is no read-then-write gap for a second departure to slip through.
///
/// **The last person out closes the plan.** An empty plan is nobody's meal, and a
/// stale link that could still seat someone into it would seat them alone into a
/// lobby whose meal, additions and time cap were chosen by people who all walked
/// out. The delete carries its own `NOT EXISTS` condition rather than trusting a
/// count read a moment earlier — the same shape as the `started_at IS NULL` above —
/// so a join landing in that gap wins and the plan survives.
async fn remove_voter(conn: &Connection, channel: &str, user: &str) -> anyhow::Result<Removed> {
    // The roster is fixed once swiping starts, in **both** directions, so this carries
    // `started_at IS NULL` in its own predicate rather than trusting a preceding read.
    // Those are two round trips and a start landing between them must win: the people
    // swiping agreed to decide together, and the number they have to agree on cannot
    // fall out from under them any more than a late joiner may raise it.
    let seat = conn
        .execute(
            "DELETE FROM pick_voters
             WHERE channel_id = ?1 AND user_id = ?2
               AND EXISTS (SELECT 1 FROM pick_sessions
                           WHERE channel_id = ?1 AND started_at IS NULL)",
            libsql::params![channel, user],
        )
        .await?;
    if seat == 0 {
        // Nothing written has two causes and they are not the same answer, so ask
        // which. A started plan refuses the departure. A seat that was simply already
        // gone is the *same* departure arriving twice — a retry after a lost response,
        // or a second tap racing the first past the handler's membership read — and it
        // has to finish like the first one rather than be told the plan started, which
        // would be a plain falsehood and would send someone hunting a start that never
        // happened. So only the started case returns here; the other falls through to
        // steps that are each conditional, and therefore already idempotent.
        let mut rows = conn
            .query(
                "SELECT 1 FROM pick_sessions
                  WHERE channel_id = ?1 AND started_at IS NOT NULL",
                libsql::params![channel],
            )
            .await?;
        if rows.next().await?.is_some() {
            return Ok(Removed::Started);
        }
    }

    // Nothing of theirs needs sweeping, and deliberately no code pretends otherwise.
    // A vote and a shopping claim each carry `started_at IS NOT NULL` in their own
    // write predicate, and the predicate above proves this plan had not started —
    // cleanup for a state the guard makes unreachable is an invitation to relax the
    // guard later, believing it is handled. A pantry pre-tick can be here, and is not
    // swept either: nobody claimed it, so it is nobody's to give up (#156, #175).

    // The last one out. This delete is the serialization point: once the plan row is
    // gone, `load_lobby` answers `None`, so every seating path (join, seat) refuses
    // before it can write another voter.
    let ended = conn
        .execute(
            "DELETE FROM pick_sessions
             WHERE channel_id = ?1
               AND NOT EXISTS (SELECT 1 FROM pick_voters WHERE channel_id = ?1)",
            libsql::params![channel],
        )
        .await?;
    if ended > 0 {
        // Nothing to sweep after it: a plan can only be emptied before it starts, so it
        // never accumulated a vote or a shopping claim, and the `NOT EXISTS` above is
        // exactly the proof that no seat is left either. A path that one day deletes a
        // *started* plan owns that cleanup; it should not be written here on spec.
        return Ok(Removed::Ended);
    }

    // Hand the plan on, but only if the person leaving is the one holding it.
    conn.execute(
        "UPDATE pick_sessions
            SET created_by = (SELECT user_id FROM pick_voters
                              WHERE channel_id = ?1
                              ORDER BY joined_at, user_id LIMIT 1)
          WHERE channel_id = ?1 AND created_by = ?2",
        libsql::params![channel, user],
    )
    .await?;

    let mut rows = conn
        .query(
            "SELECT created_by FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    // No row means a concurrent departure emptied it between the two statements —
    // the plan is gone either way, which is what `Ended` says.
    Ok(match rows.next().await? {
        Some(r) => Removed::Left {
            host: r.get::<String>(0)?,
        },
        None => Removed::Ended,
    })
}

/// Close the lobby and begin the pick. Idempotent, and deliberately keeps the first
/// start time: pressing start twice must not move the moment the roster closed.
async fn begin_session(conn: &Connection, channel: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE pick_sessions SET started_at = unixepoch()
         WHERE channel_id = ?1 AND started_at IS NULL",
        libsql::params![channel],
    )
    .await?;
    Ok(())
}

/// A plan's lobby, or `None` if no such plan exists.
async fn load_lobby(conn: &Connection, channel: &str) -> anyhow::Result<Option<LobbyView>> {
    let mut rows = conn
        .query(
            "SELECT created_by, kitchen_id, started_at, meal_type, additions, max_total_seconds,
                    decided_source, decided_id, decided_at, seed, created_at
             FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let host: String = row.get(0)?;
    let kitchen_id: Option<String> = row.get(1)?;
    let started_at: Option<i64> = row.get(2)?;
    // Every writer of these two columns validates against its tier's vocabulary,
    // so a stored word outside it is corruption — fail loud rather than serve a
    // plan for a meal that does not exist (the db.rs lesson: a wrong database
    // must not run beautifully).
    let meal_raw: String = row.get(3)?;
    let meal_type = MealType::parse(&meal_raw).ok_or_else(|| {
        anyhow::anyhow!("pick_sessions.meal_type outside the vocabulary: {meal_raw:?}")
    })?;
    let additions_raw: String = row.get(4)?;
    let additions: Vec<MealAddition> = serde_json::from_str(&additions_raw).map_err(|e| {
        anyhow::anyhow!("pick_sessions.additions outside the vocabulary: {additions_raw:?}: {e}")
    })?;
    let max_total_seconds: Option<i64> = row.get(5)?;
    // Read from this row rather than by a second query, so the lobby's `started` and
    // its `decided` can never describe two different instants of the same plan.
    let decided = decision_of(row.get(6)?, row.get(7)?, row.get(8)?)?;
    // Read from this same row for the same reason the decision is: a plan's seed and its
    // birth instant are the pair every derived-shared thing is computed from, and reading
    // them separately would be two reads that could describe two plans.
    let seed: Option<i64> = row.get(9)?;
    let created_at: i64 = row.get(10)?;

    let mut vrows = conn
        .query(
            "SELECT v.user_id, u.username
             FROM pick_voters v
             LEFT JOIN users u ON u.telegram_user_id = v.user_id
             WHERE v.channel_id = ?1
             ORDER BY v.joined_at, v.user_id",
            libsql::params![channel],
        )
        .await?;
    let mut voters = Vec::new();
    while let Some(v) = vrows.next().await? {
        voters.push(Voter {
            telegram_user_id: v.get::<String>(0)?,
            username: v.get::<Option<String>>(1)?,
        });
    }

    // The pool the host can pull in without a link: this kitchen's members who are not
    // already deciding. A plan with no kitchen has no such pool — it is invite-only.
    let candidates = match &kitchen_id {
        Some(kid) => {
            let seated: std::collections::HashSet<&str> =
                voters.iter().map(|v| v.telegram_user_id.as_str()).collect();
            crate::kitchens::member_list(conn, kid)
                .await?
                .into_iter()
                .filter(|(id, _)| !seated.contains(id.as_str()))
                .map(|(telegram_user_id, username)| Voter {
                    telegram_user_id,
                    username,
                })
                .collect()
        }
        None => Vec::new(),
    };

    Ok(Some(LobbyView {
        channel_id: channel.to_owned(),
        kitchen_id,
        meal_type,
        additions,
        host,
        started: started_at.is_some(),
        max_total_seconds,
        voters,
        candidates,
        decided,
        seed,
        created_at,
    }))
}

async fn session_exists(conn: &Connection, channel: &str) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// Insert a new session. `channel_id` is unique (the primary key).
///
/// The parameter list mirrors the INSERT's column list one-for-one — a struct
/// here would relabel the same seven values without making any call site
/// clearer (the same trade `derive.rs` makes).
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    conn: &Connection,
    channel_id: &str,
    created_by: &str,
    filter: Option<&str>,
    kitchen_id: Option<&str>,
    meal_type: MealType,
    additions: &[MealAddition],
    max_total_seconds: Option<i64>,
) -> anyhow::Result<()> {
    conn.execute(
        // The seed is minted here and never again: it is a fact about the plan's birth,
        // like `created_at` beside it, and a plan whose seed could change is a plan whose
        // room could be dealt two different sequences of the same thing.
        "INSERT INTO pick_sessions
            (channel_id, created_by, filter, kitchen_id, meal_type, additions,
             max_total_seconds, seed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        libsql::params![
            channel_id,
            created_by,
            filter,
            kitchen_id,
            meal_type.as_str(),
            serde_json::to_string(&normalize_additions(additions))?,
            max_total_seconds,
            mint_seed()
        ],
    )
    .await?;
    Ok(())
}

/// Set (or lift, with `None`) a plan's time cap (#80), reporting whether it was
/// written.
///
/// `started_at IS NULL` is in the predicate rather than only in the handler's earlier
/// read: those are two round trips, and a `start()` landing between them would
/// otherwise move the corpus bound out from under a plan already being swiped. Here
/// the loser of that race writes nothing and says so.
async fn set_time_cap(
    conn: &Connection,
    channel: &str,
    max_total_seconds: Option<i64>,
) -> anyhow::Result<bool> {
    let written = conn
        .execute(
            "UPDATE pick_sessions SET max_total_seconds = ?2
             WHERE channel_id = ?1 AND started_at IS NULL",
            libsql::params![channel, max_total_seconds],
        )
        .await?;
    Ok(written > 0)
}

/// Everything about a plan that bounds the walk it deals (#80, #82, #184).
///
/// One struct and one read, rather than a query per facet: the walk resolves the whole
/// bound from the channel on every call, and each facet (#80's cap, #82's kitchen,
/// #184's meal) would otherwise be another round trip on the pick page's hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanBounds {
    /// The total-time cap in seconds (#80); `None` = "Any".
    pub max_total_seconds: Option<i64>,
    /// The kitchen this plan is for (#72), whose equipment limits what the walk deals
    /// (#82); `None` for a plan started outside a kitchen. Unconditional — there is no
    /// flag, because a meal planned in a kitchen is cooked in that kitchen.
    pub kitchen_id: Option<String>,
    /// Which meal this plan is for (#114). The pick's one round deals *the meal*, so
    /// the walk keeps that round clear of dishes the corpus states are accompaniments
    /// (#184) — the choice a host makes in the lobby reaching the deck at last.
    ///
    /// Not an `Option`: migration 0016 made the column `NOT NULL DEFAULT 'dinner'` and
    /// the create handler applies the same default, so every plan is for some meal.
    pub meal_type: MealType,
}

/// The bounds of a plan that named nothing: no cap, no kitchen, and the meal every plan
/// is born for.
///
/// Written out rather than derived, because [`MealType`] is the corpus's vocabulary
/// (#191) and a *dish* has no default sitting — only a plan does. Deriving `Default`
/// would have needed one on the shared type, where it could quietly stand in for a
/// reading that should have been refused as empty.
impl Default for PlanBounds {
    fn default() -> Self {
        PlanBounds {
            max_total_seconds: None,
            kitchen_id: None,
            meal_type: DEFAULT_MEAL_TYPE,
        }
    }
}

/// A session's bounds, for the walk: `Ok(None)` is an unknown session, `Ok(Some(..))`
/// its bounds. The two layers are deliberate — an unknown channel must surface as an
/// error to the caller, never read as "unbounded", which would hand a mistyped channel
/// the whole corpus (#80).
pub async fn plan_bounds(conn: &Connection, channel: &str) -> anyhow::Result<Option<PlanBounds>> {
    let mut rows = conn
        .query(
            "SELECT max_total_seconds, kitchen_id, meal_type
             FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    // Same ruling as [`load_lobby`]: every writer of this column validates against the
    // vocabulary, so a stored word outside it is corruption. Fail loud rather than
    // quietly fall back to the default and deal a deck bounded by a meal nobody chose.
    let meal_raw: String = row.get(2)?;
    let meal_type = MealType::parse(&meal_raw).ok_or_else(|| {
        anyhow::anyhow!("pick_sessions.meal_type outside the vocabulary: {meal_raw:?}")
    })?;
    Ok(Some(PlanBounds {
        max_total_seconds: row.get(0)?,
        kitchen_id: row.get(1)?,
        meal_type,
    }))
}

/// Point a plan at a different meal (#114), reporting whether it was written. The word
/// is already validated and typed; the `started_at IS NULL` predicate is the same
/// race guard as [`set_time_cap`] — what a plan is for cannot change once it is under
/// way.
async fn update_meal_type(
    conn: &Connection,
    channel: &str,
    meal_type: MealType,
) -> anyhow::Result<bool> {
    let written = conn
        .execute(
            "UPDATE pick_sessions SET meal_type = ?2
             WHERE channel_id = ?1 AND started_at IS NULL",
            libsql::params![channel, meal_type.as_str()],
        )
        .await?;
    Ok(written > 0)
}

/// Replace what comes with the meal (#114), reporting whether it was written. The set
/// is already validated and typed and lands in its canonical form; the
/// `started_at IS NULL` predicate is the same race guard as [`set_time_cap`].
async fn update_additions(
    conn: &Connection,
    channel: &str,
    additions: &[MealAddition],
) -> anyhow::Result<bool> {
    let written = conn
        .execute(
            "UPDATE pick_sessions SET additions = ?2
             WHERE channel_id = ?1 AND started_at IS NULL",
            libsql::params![
                channel,
                serde_json::to_string(&normalize_additions(additions))?
            ],
        )
        .await?;
    Ok(written > 0)
}

/// The precondition every vote and every shopping claim is written under (#175):
/// **a seat at a plan whose swiping has begun**.
///
/// It is a SQL fragment rather than a Rust check because it belongs *inside* the
/// write, alongside [`remove_voter`]'s `started_at IS NULL`. The two halves are here
/// for different reasons, and neither is quite the other's:
///
/// - **The roster.** Nothing asked at all before #175 — the socket upgrade gates on the
///   channel existing — so a signed-in stranger holding the invite link wrote into the
///   very tally the room is measured by. Since #209 the framework asks first
///   ([`is_seated_in_a_started_plan`], at [`crate::events::ingest`]'s choke point), and
///   that is a *different* question with a different job: it decides whether a handler
///   runs at all, and this decides whether the row lands. Two round trips separate them,
///   and a seat given up in between must win.
/// - **The start.** Not a race. `started_at` only ever goes NULL → set, so a caller
///   that saw "started" still sees it when the write lands. What this buys is the
///   sentence #169 wrote in [`remove_voter`] — "votes and shopping claims only exist
///   after the start" — made true by the database rather than by the browser, which
///   is what makes the absent sweeps sound.
///
/// Together they close each other's remaining gap, and that is why they go in
/// together: a seat can only be given up before the start, and neither table may be
/// written before it, so the roster a write is judged against can no longer move
/// underneath one. Each is still asked on its own, because a guard that holds only
/// while a *different* guard holds is the kind that quietly stops holding.
///
/// Deliberately **not** applied to the pantry seed ([`write_seed`]): a pre-tick is
/// nobody's claim — it is a function of the plan's kitchen and the recipe, reachable
/// from the ungated read ([`buy_list`]) precisely because it depends on nothing about
/// the caller. Nobody's claim is nobody's to lose, which is why [`remove_voter`] still
/// has nothing to sweep.
///
/// `person` is the **placeholder** the caller bound the telegram id to (`"?4"`), not
/// an id — the channel is always `?1`, and the person's position differs between the
/// statements below. Every argument it is ever given is a literal written here.
pub(crate) fn seated_in_a_started_plan(person: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM pick_sessions s, pick_voters v
                  WHERE s.channel_id = ?1 AND s.started_at IS NOT NULL
                    AND v.channel_id = ?1 AND v.user_id = {person})"
    )
}

/// The same predicate, asked as a question — the event framework's guard
/// (`events::Guard::SeatedInStartedPlan`) applied at its choke point.
///
/// Built out of [`seated_in_a_started_plan`] rather than beside it, so there is exactly
/// one description of who may write to a plan and a change to it reaches the framework
/// and every existing write at once. Asking it here does **not** retire the copy inside
/// each write's own predicate: this decides whether a handler runs at all, and the
/// write's predicate is what makes the answer race-free when a seat is given up in the
/// round trip between the two (#175/#179).
pub(crate) async fn is_seated_in_a_started_plan(
    conn: &Connection,
    channel: &str,
    person: &str,
) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            &format!("SELECT 1 WHERE {}", seated_in_a_started_plan("?2")),
            libsql::params![channel, person],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// **The plan has decided** (#201) — the precondition of everything that happens after
/// the pick.
///
/// Written once and read in both directions: as itself by the cook's guard (#211, which
/// is the whole of "you cook the decision"), and negated by
/// [`in_an_undecided_plan`] below for the vote that may no longer be cast. One
/// description of a plan's decision means a change to it cannot reach one side and miss
/// the other, and it means the two can never drift into disagreeing about what a decided
/// plan is.
///
/// The channel is always `?1`, so this fragment takes no argument.
fn in_a_decided_plan() -> &'static str {
    "EXISTS (SELECT 1 FROM pick_sessions s
              WHERE s.channel_id = ?1 AND s.decided_at IS NOT NULL)"
}

/// The other half of a vote's precondition since #201: **the plan has not decided**.
///
/// A decided plan's deck is over, so a swipe arriving after it is refused rather than
/// counted — the honest answer, because the alternative is a tally that keeps moving
/// under a fact that cannot. It is written here rather than folded into
/// [`seated_in_a_started_plan`] because that fragment is shared with the shopping
/// writes, and shopping is what happens *after* the decision: one predicate covering
/// both would make the decision close the shop it opened.
///
/// This one **is** a race, unlike the start. `decided_at` goes NULL → set exactly once
/// and never back, so a socket that read "undecided" a round trip ago may be writing
/// into a plan that has since decided; the guard has to be inside the insert, which is
/// the #169/#179 discipline said for the last state a vote can arrive too late for.
fn in_an_undecided_plan() -> String {
    format!("NOT {}", in_a_decided_plan())
}

/// Record (or update) a voter's call on a recipe, reporting whether it was written.
/// Re-voting overwrites — a swipe is a current decision, not an append.
///
/// Only a decider in a started, **undecided** plan may write one, and both halves live
/// in the insert's own predicate ([`seated_in_a_started_plan`],
/// [`in_an_undecided_plan`]). A vote is not a private note: the tally is read as `yes`
/// against the *roster*, so a yes from outside the roster completes a consensus the
/// people deciding never reached. Nothing before this checked — the socket upgrade asks
/// only that the channel exist — so a signed-in stranger holding the invite link voted
/// into somebody else's dinner.
///
/// A refused vote is silent, and that is the same silence #179/#180 already described:
/// a vote is a socket frame the server never answers, so there is nothing to report a
/// refusal *through*. What is not silent is the reason — a decided plan has already
/// broadcast [`ServerMsg::Decided`] to the whole room and re-sends it on every connect,
/// so a client swiping into a decided plan is a client that has been told, or is about
/// to be, by the frame that ends its deck.
///
/// **`at` is the swipe's own instant** (#209), on the shared timeline: the moment the
/// card went left or right, taken from the initiator's clock and corrected for that
/// participant's measured drift by [`crate::events::normalize`] before it reaches here.
/// This function reads no clock, exactly as [`crate::timers::start`] reads none — the
/// tap is the event, and the moment the row happened to be written is a fact about the
/// network. `created_at` still records that second-resolution write time and is
/// untouched; `created_at_ms` is the new column beside it (migration 0028), and it moves
/// with a re-vote because a swipe is a person's *current* call.
pub(crate) async fn record_vote(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    voter: &str,
    vote: bool,
    at: i64,
) -> anyhow::Result<bool> {
    let written = conn
        .execute(
            &format!(
                "INSERT INTO votes (channel_id, source, id, voter_id, vote, created_at_ms)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE {} AND {}
                 ON CONFLICT(channel_id, source, id, voter_id) DO UPDATE SET
                    vote = excluded.vote,
                    created_at = unixepoch(),
                    created_at_ms = excluded.created_at_ms",
                seated_in_a_started_plan("?4"),
                in_an_undecided_plan()
            ),
            libsql::params![channel, source, id, voter, vote as i64, at],
        )
        .await?;
    Ok(written > 0)
}

// ---- the decision (#201) ---------------------------------------------------

/// Evaluate the pick's win condition for one recipe and, if it is met, **record** the
/// decision — in a single statement, so the condition is the write's own predicate.
///
/// Returns the decision when *this* call is the one that recorded it, and `None`
/// otherwise: not met, or somebody else got there first. So the caller does not have
/// to ask a second question to know whether to announce anything, and exactly one
/// [`ServerMsg::Decided`] is ever broadcast per plan.
///
/// **The condition, and why each clause is asked separately.**
///
/// - `started_at IS NOT NULL` — a lobby decides nothing. No vote can exist before the
///   start ([`seated_in_a_started_plan`]) so this is already true; it is asked anyway,
///   for the reason #175 gives: a guard that holds only while a *different* guard holds
///   is the kind that quietly stops holding when the other one moves.
/// - `decided_at IS NULL` — **first past the post.** Two votes completing at the same
///   instant both run this UPDATE; the predicate is re-evaluated inside each write, so
///   exactly one changes a row. The loser sees zero rows and announces nothing. This is
///   the clause that makes the record immutable: a decision, once made, is never
///   overwritten by a later one, however the tally moves afterwards.
/// - the roster is not empty — arithmetic, not paranoia. With no deciders the count
///   equality below reads `0 = 0` and every recipe with no votes would be "agreed".
///   A plan always holds at least its host (#96 deletes an emptied one), so this can
///   only fire on a plan that no longer exists.
/// - no decider said **no** — one veto is enough, which is what "everyone likes it"
///   means. **Redundant today, and kept**: `votes` is keyed
///   `(channel_id, source, id, voter_id)`, so a person holds one row per recipe and is
///   either a yes or a no on it, never both. The count below can therefore only reach
///   the roster size when every decider's single row is a yes — which already means
///   nobody said no. Dropping this clause changes no answer that any state of the two
///   tables can produce (an *equivalent* mutation, not an untested one), and it stays
///   because the rule has two halves and the SQL should say both: the second half would
///   otherwise be true only by an argument about a primary key two migrations away, and
///   a `votes` that ever became append-only would silently start deciding over vetoes.
///   `a_persons_vote_is_one_row_so_a_yes_and_a_no_cannot_coexist` pins the key the
///   redundancy rests on.
/// - the distinct deciders who said **yes** number exactly the roster.
///
/// **Both counts are taken over the roster**, joined to `pick_voters` rather than read
/// off `votes` alone. Since #179 nothing else can be in that table, so the join changes
/// no answer today — but the ruling is that the roster is the deciding body, and saying
/// that once in each direction is what stops a row predating #179 either completing a
/// consensus nobody reached or vetoing one everybody did. A voter has at most one row
/// per recipe (the `votes` primary key), so counting rows *is* counting people.
///
/// **`at` is the deciding swipe's own instant** (#209), and the whole win condition is
/// still inside this one predicate — nothing about the migration onto the event
/// framework moved a clause out, relaxed one, or split the evaluation from the write.
/// `decided_at_ms` is set from it (migration 0028); `decided_at` is still
/// `unixepoch()` and is still the column `decided_at IS NULL` is asked about, so what
/// makes a decision first-past-the-post is untouched and the new column can never become
/// a second answer to *whether* a plan decided.
///
/// A decision is the one event nobody raises: it is a consequence of the last yes,
/// evaluated inside that vote's write, so its instant *is* that vote's instant. Stamping
/// it with a clock read here would say the plan decided when the UPDATE ran, which is a
/// fact about the network between the deciding phone and this process.
pub(crate) async fn decide_if_agreed(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    at: i64,
) -> anyhow::Result<Option<DecidedRecipe>> {
    let mut rows = conn
        .query(
            "UPDATE pick_sessions
                SET decided_source = ?2, decided_id = ?3, decided_at = unixepoch(),
                    decided_at_ms = ?4
              WHERE channel_id = ?1
                AND started_at IS NOT NULL
                AND decided_at IS NULL
                AND EXISTS (SELECT 1 FROM pick_voters r WHERE r.channel_id = ?1)
                AND NOT EXISTS (
                      SELECT 1 FROM votes v
                        JOIN pick_voters r
                          ON r.channel_id = v.channel_id AND r.user_id = v.voter_id
                       WHERE v.channel_id = ?1 AND v.source = ?2 AND v.id = ?3
                         AND v.vote = 0)
                AND (SELECT COUNT(*) FROM votes v
                       JOIN pick_voters r
                         ON r.channel_id = v.channel_id AND r.user_id = v.voter_id
                      WHERE v.channel_id = ?1 AND v.source = ?2 AND v.id = ?3
                        AND v.vote = 1)
                  = (SELECT COUNT(*) FROM pick_voters r WHERE r.channel_id = ?1)
              RETURNING decided_source, decided_id, decided_at",
            libsql::params![channel, source, id, at],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    decision_of(row.get(0)?, row.get(1)?, row.get(2)?)
}

/// The three decision columns of one `pick_sessions` row, read as one fact.
///
/// All three are written by one statement ([`decide_if_agreed`]) or by none, so the
/// only two shapes that exist are all-set and all-null. Anything else is corruption and
/// is refused **loudly**, the same ruling [`load_lobby`] applies to a `meal_type`
/// outside the vocabulary: a plan is about to be told what it decided, and half a
/// decision — a recipe with no time on it, or a time with no recipe — is exactly the
/// kind of wrong database that must not run beautifully. SQLite cannot add a CHECK with
/// `ALTER TABLE ADD COLUMN`, so this is where the invariant is asserted instead (see
/// migration 0026 for why the table was not rebuilt to gain one).
fn decision_of(
    source: Option<String>,
    id: Option<String>,
    decided_at: Option<i64>,
) -> anyhow::Result<Option<DecidedRecipe>> {
    match (source, id, decided_at) {
        (Some(source), Some(id), Some(decided_at)) => Ok(Some(DecidedRecipe {
            source,
            id,
            decided_at,
        })),
        (None, None, None) => Ok(None),
        (source, id, decided_at) => Err(anyhow::anyhow!(
            "pick_sessions holds half a decision: \
             decided_source={source:?} decided_id={id:?} decided_at={decided_at:?}"
        )),
    }
}

/// What a plan decided (#201), or `None` if it has not — and `None` too for a channel
/// that does not exist, which every caller has already established by the time it asks.
async fn load_decision(conn: &Connection, channel: &str) -> anyhow::Result<Option<DecidedRecipe>> {
    let mut rows = conn
        .query(
            "SELECT decided_source, decided_id, decided_at
             FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    decision_of(row.get(0)?, row.get(1)?, row.get(2)?)
}

// ---- the cook (#211) -------------------------------------------------------

/// **A plan that is cooking**: when the cook started, and whose tap started it.
///
/// The room's own record of the moment "Let's cook!" was pressed, read back for the
/// [`ServerMsg::Cooking`] frame — live, and again on every connect. It carries no recipe
/// for the reason the frame does not: a plan cooks the meal it decided, which is already
/// a recorded fact ([`DecidedRecipe`]), and a second copy of it here would be a second
/// answer to what the room is having.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cooking {
    /// The initiator's tap, in the shared timeline (unix ms) — see migration 0029.
    pub started_at: i64,
    /// Whose it was, as a whole person.
    pub started_by: Voter,
}

/// The framework's guard for the cook (`events::Guard::SeatedInDecidedPlan`), asked at
/// its one choke point: **on the roster of a started plan, and that plan has decided.**
///
/// Built out of [`seated_in_a_started_plan`] and [`in_a_decided_plan`] rather than beside
/// them, so there is one description of each half and a change to either reaches the
/// framework and every write at once.
///
/// **You cook the decision, so the decision is part of who may start a cook**, and that
/// is the design choice this predicate states. A plan still swiping has nothing to cook:
/// there is no recipe, no shopping list keyed to one (`not_against_the_decision`), and no
/// screen at the other end of the transition — so a cook raised there could only move the
/// room to a stove with no pot on it. The framework already had a guard for "may write to
/// this plan at all"; this is the second one it has ever needed, which is the test of
/// [`crate::events::Guard`] naming a predicate about a person and a plan rather than a
/// feature.
pub(crate) async fn is_seated_in_a_decided_plan(
    conn: &Connection,
    channel: &str,
    person: &str,
) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            &format!(
                "SELECT 1 WHERE {} AND {}",
                seated_in_a_started_plan("?2"),
                in_a_decided_plan()
            ),
            libsql::params![channel, person],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// **Record that this plan is cooking**, reporting whether *this* call is what recorded
/// it.
///
/// `at` is the tap's own instant on the shared timeline, from the envelope — this
/// function reads no clock, exactly as [`crate::timers::start`] and [`record_vote`] read
/// none. The tap is the event; the moment the row happened to be written is a fact about
/// the network.
///
/// **Idempotent, in two halves, and both are load-bearing.**
///
/// - `cook_started_at_ms IS NULL` is inside the write, so the **first** tap is the one
///   recorded and no later tap moves it. That is first-past-the-post, exactly as
///   [`decide_if_agreed`]'s `decided_at IS NULL` is: two people tapping at the same
///   instant both run this UPDATE and exactly one changes a row. A cook has one start,
///   and a second tap must not restate it as five minutes later than it was — a plan that
///   re-anchored its own start every time somebody pressed a button would be lying about
///   when dinner went on.
/// - The **announcement does not depend on the write** ([`crate::events::apply`] re-reads
///   and announces whatever is recorded). So a second tap is a no-op that is still
///   answered with the truth, which is the [`ServerMsg::Timers`]/[`ServerMsg::Buy`] rule
///   — a whole-state frame is self-healing, and re-stating a fact is the cheapest way to
///   be right. That is what makes a second tap **a no-op rather than an error**: nothing
///   moves, nobody is refused, and the tapper's screen is carried to the stove by the
///   same frame that carried everybody else's.
///
/// The roster, the start and the decision are all in the predicate here as well as at the
/// framework's choke point ([`is_seated_in_a_decided_plan`]), which is the #175/#179
/// discipline: that read decides whether the handler runs at all, and this predicate is
/// what makes the answer race-free when a seat is given up — or a plan decides — in the
/// round trip between the two.
pub(crate) async fn start_cook(
    conn: &Connection,
    channel: &str,
    user: &str,
    at: i64,
) -> anyhow::Result<bool> {
    let written = conn
        .execute(
            &format!(
                "UPDATE pick_sessions
                    SET cook_started_at_ms = ?3, cook_started_by = ?2
                  WHERE channel_id = ?1
                    AND cook_started_at_ms IS NULL
                    AND {} AND {}",
                seated_in_a_started_plan("?2"),
                in_a_decided_plan()
            ),
            libsql::params![channel, user, at],
        )
        .await?;
    Ok(written > 0)
}

/// Whether this plan is cooking, and since when — `None` while it is not, and `None` too
/// for a channel that does not exist.
///
/// The username is joined here rather than looked up by whoever renders it, the way
/// [`crate::timers::load`] joins the person who started a countdown: the frame says whose
/// cook it is, so the read that builds it is where the name comes from.
pub(crate) async fn load_cook(conn: &Connection, channel: &str) -> anyhow::Result<Option<Cooking>> {
    let mut rows = conn
        .query(
            "SELECT s.cook_started_at_ms, s.cook_started_by, u.username
             FROM pick_sessions s
             LEFT JOIN users u ON u.telegram_user_id = s.cook_started_by
             WHERE s.channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    cook_of(row.get(0)?, row.get(1)?, row.get::<Option<String>>(2)?)
}

/// The two cook columns of one `pick_sessions` row, read as one fact.
///
/// Both are written by one statement ([`start_cook`]) or by neither, so the only two
/// shapes that exist are both-set and both-null. Anything else is corruption and is
/// refused **loudly**, the same ruling [`decision_of`] applies to half a decision and
/// [`load_lobby`] applies to a `meal_type` outside the vocabulary: a plan cooking with
/// nobody's hand on it — or somebody cooking at no time — is exactly the kind of wrong
/// database that must not run beautifully. SQLite cannot add a CHECK with
/// `ALTER TABLE ADD COLUMN`, so this is where the invariant is asserted instead.
fn cook_of(
    started_at: Option<i64>,
    started_by: Option<String>,
    username: Option<String>,
) -> anyhow::Result<Option<Cooking>> {
    match (started_at, started_by) {
        (Some(started_at), Some(telegram_user_id)) => Ok(Some(Cooking {
            started_at,
            started_by: Voter {
                telegram_user_id,
                username,
            },
        })),
        (None, None) => Ok(None),
        (started_at, started_by) => Err(anyhow::anyhow!(
            "pick_sessions holds half a cook: \
             cook_started_at_ms={started_at:?} cook_started_by={started_by:?}"
        )),
    }
}

/// The winning recipe of a kitchen's meal (#207): what a plan decided, **named**.
///
/// [`DecidedRecipe`] is the same fact for the plan's own screens, and carries
/// `decided_at` instead of a title because the pick page already holds the card it is
/// about. A kitchen listing holds nothing: it is looking at a plan it was never in, so
/// the pair `(source, id)` on its own would be a row saying a meal was decided and
/// refusing to say what for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecidedMeal {
    pub source: String,
    pub id: String,
    /// The recipe's `recipes.title`, joined in the read below rather than fetched per
    /// row by whoever renders it.
    pub title: String,
}

/// One meal of a kitchen (#207) — a plan on `pick_sessions.kitchen_id`, as a list of
/// them shows it.
///
/// Not a [`LobbyView`]: that is the plan's own screen and carries everything a person
/// deciding needs — the host, the roster by name, the kitchen's seatable members, the
/// time cap. A kitchen lists meals it is not in the middle of, so it carries what tells
/// them apart at a glance and nothing else. The roster arrives as a **count** for the
/// same reason: how many are in is the fact a list is asking about, and six names per
/// row would be six lookups per row for a page that is not the lobby.
///
/// Four states, and they are exactly the four the columns can be in: gathering
/// (`started` false), deciding (`started` true, no decision), decided, and **cooking**
/// (#211 — decided, and somebody has started the cook). A plan everybody walked out of is
/// a deleted row (#96/#169), so an "over" state is not something this can hold — such a
/// meal is simply not in the list.
///
/// The fourth arrived for free: it is one more column of the row this read already
/// fetches, on no new join and no second query, so a kitchen can say which of its meals
/// is on the hob for exactly what the other three cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KitchenMeal {
    /// The plan's channel — what `/pick/{channel}` is, so the row is a link.
    pub channel_id: String,
    /// Which meal this plans (#114), and what comes with it — the words the plan was
    /// made in, so a list of meals reads as meals.
    pub meal_type: MealType,
    pub additions: Vec<MealAddition>,
    /// Whether the swiping has begun, which is the line between gathering and deciding.
    pub started: bool,
    /// How many people are on the roster — who joined the plan, not who is connected
    /// and not who has voted (the same number [`ServerMsg::Lobby`] carries).
    pub deciders: i64,
    /// What this meal decided (#201/#205), or `None` while its deck is still running.
    pub decided: Option<DecidedMeal>,
    /// Whether somebody has started the cook (#211).
    ///
    /// Only ever `true` beside a `decided`, because the cook's guard is "seated in a
    /// decided plan" and a decision never goes back to none — a row that says otherwise
    /// is refused loudly in [`kitchen_meals`] rather than listed. It is a `bool` and not
    /// the instant, because a list is asking *which* meal is on the hob and not for how
    /// long; the plan's own screens read [`Cooking`] for that.
    pub cooking: bool,
}

/// A kitchen's meals (#207), newest first.
///
/// **One query.** The three things a row needs beyond the plan itself — its roster
/// size, and the title of what it decided — are a correlated count and a left join
/// against `recipes`, so a kitchen with a dozen meals costs one round trip rather than
/// a dozen title fetches from a browser. The join is keyed on the decision's own two
/// columns, which is the same `(source, id)` pair the corpus is keyed by everywhere
/// (`walk.rs` reads a card by it, `buy` reads a recipe by it).
///
/// The join is a LEFT one because most plans have decided nothing: an inner join would
/// list only the finished meals, which is the opposite of what a kitchen is for. A
/// decision whose recipe is *missing* is refused loudly instead of rendered as a plan
/// with no outcome — `recipes` is derived by upsert and nothing deletes from it, so a
/// decided pair with no row is corruption, and #205 made a decision a server fact
/// precisely so that nobody has to guess at one.
///
/// `created_at` orders it, because that is when the meal was called. Newest first: a
/// kitchen's meals accumulate and the one somebody is looking for is the one they are
/// having. `channel_id` breaks the tie — `created_at` is whole seconds, so two plans
/// called in the same second would otherwise come back in whatever order the storage
/// engine felt like, and a list that reshuffles between reads is its own bug.
pub async fn kitchen_meals(
    conn: &Connection,
    kitchen_id: &str,
) -> anyhow::Result<Vec<KitchenMeal>> {
    let mut rows = conn
        .query(
            "SELECT s.channel_id, s.meal_type, s.additions, s.started_at,
                    (SELECT COUNT(*) FROM pick_voters v WHERE v.channel_id = s.channel_id),
                    s.decided_source, s.decided_id, s.decided_at, r.title,
                    s.cook_started_at_ms
             FROM pick_sessions s
             LEFT JOIN recipes r
                    ON r.source = s.decided_source AND r.id = s.decided_id
             WHERE s.kitchen_id = ?1
             ORDER BY s.created_at DESC, s.channel_id DESC",
            libsql::params![kitchen_id.to_owned()],
        )
        .await?;

    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let channel_id: String = row.get(0)?;
        // Both vocabularies are validated by every writer, so a stored word outside
        // one is corruption — the same loud refusal `load_lobby` makes, for the same
        // reason: a meal the app does not have is not a meal to list.
        let meal_raw: String = row.get(1)?;
        let meal_type = MealType::parse(&meal_raw).ok_or_else(|| {
            anyhow::anyhow!("pick_sessions.meal_type outside the vocabulary: {meal_raw:?}")
        })?;
        let additions_raw: String = row.get(2)?;
        let additions: Vec<MealAddition> = serde_json::from_str(&additions_raw).map_err(|e| {
            anyhow::anyhow!(
                "pick_sessions.additions outside the vocabulary: {additions_raw:?}: {e}"
            )
        })?;
        let started_at: Option<i64> = row.get(3)?;
        let deciders: i64 = row.get(4)?;
        // The same classifier the plan's own reads use, so "half a decision" is one
        // rule in one place; the title is the half only this read needs.
        let decision = decision_of(row.get(5)?, row.get(6)?, row.get(7)?)?;
        let title: Option<String> = row.get(8)?;
        let decided = match (decision, title) {
            (Some(d), Some(title)) => Some(DecidedMeal {
                source: d.source,
                id: d.id,
                title,
            }),
            (Some(d), None) => {
                return Err(anyhow::anyhow!(
                    "plan {channel_id} decided {}/{}, which the corpus does not hold",
                    d.source,
                    d.id
                ))
            }
            (None, _) => None,
        };
        // The fourth word (#211), off one more column of the row already in hand. A cook
        // on a plan that decided nothing is refused the way a decision the corpus cannot
        // name is: `start_cook`'s own predicate makes it unwritable, so a row holding one
        // is corruption rather than a state to render — and rendering it would be a
        // kitchen offering a way into a stove with no pot named on it.
        let cooking: Option<i64> = row.get(9)?;
        if cooking.is_some() && decided.is_none() {
            return Err(anyhow::anyhow!(
                "plan {channel_id} is cooking a meal it never decided"
            ));
        }

        out.push(KitchenMeal {
            channel_id,
            meal_type,
            additions,
            started: started_at.is_some(),
            deciders,
            decided,
            cooking: cooking.is_some(),
        });
    }
    Ok(out)
}

/// The predicate every write on a shopping list carries since #201: **this recipe is
/// not one the plan decided against**.
///
/// Phrased as "no recorded decision contradicts it" rather than "the decision names
/// it", and the difference is the whole transition story. A plan that has decided
/// admits writes for exactly one recipe. A plan that has not decided admits them for
/// any, which is what the handlers did before this and is what keeps a shopping list
/// stashed in a browser before this deployed working — migration 0026 backfills nothing
/// and could not honestly have.
///
/// It is in the write and not only in the handler's read because `decided_at` goes
/// NULL → set: a caller that read "undecided" can have the decision land underneath it,
/// and the loser of that race must write nothing rather than build a list for a recipe
/// the plan turned down. The read handler still asks ([`decided_recipe_or_refuse`]),
/// because that is what gives *its* refusal a sentence — the same division of labour
/// [`seated_in_a_started_plan`] has between the framework's guard and each write.
///
/// `source` and `id` are the **placeholders** the caller bound them to; they are `?2`
/// and `?3` on every statement below, but naming them keeps the fragment honest about
/// what it reads. The channel is always `?1`.
pub(crate) fn not_against_the_decision(source: &str, id: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM pick_sessions s
                      WHERE s.channel_id = ?1 AND s.decided_at IS NOT NULL
                        AND (s.decided_source <> {source} OR s.decided_id <> {id}))"
    )
}

/// Claim one line of a meal's shopping list for `user` (#131).
///
/// Idempotent, and a take-over rather than a duplicate: the primary key does not
/// include the person, so a second tapper replaces the first (last writer wins) and
/// the timestamp moves with them. Tapping your own tick again rewrites the same row
/// to the same values.
///
/// `pantry_item` is cleared on the way through (#156). A pantry pre-tick is a claim
/// nobody made; the moment somebody taps that line it becomes theirs, and a row
/// carrying both would be two answers to "who has this". The CHECK in migration 0021
/// would refuse it anyway.
///
/// The roster and the start are in the insert's own predicate
/// ([`seated_in_a_started_plan`]), not only in the framework's preceding guard
/// ([`crate::events::ingest`]): the roster this is judged against is the one that exists
/// when the write lands, exactly as [`remove_voter`]'s seat delete is judged against the
/// start that exists when *it* lands. Since #201 the **recipe** is judged there too
/// ([`not_against_the_decision`]): a shopping list belongs to the meal the plan
/// decided, and the query string is no longer the only thing that says which that is.
///
/// **A negative index writes nothing**, and that lives in the predicate for the same
/// reason everything else here does (#209). It used to be a sentence the HTTP handler
/// said before writing; the handler is gone, and a check that lived only in a caller is
/// a check the next caller can miss. An index is a position in the recipe's shopping-list
/// projection, so a negative one names no line of any recipe — a row for it would be
/// unreachable by every read and untickable by every screen.
///
/// **`at` is the tap's own instant** (#209), on the shared timeline, and it moves with
/// the line the way `user_id` does: a take-over records when the new hand closed on it,
/// not when the first did. `created_at` still records the row's `unixepoch()` write time.
pub(crate) async fn tick_item(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    index: i64,
    user: &str,
    at: i64,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO buy_checks
                (channel_id, source, id, ingredient_index, user_id, pantry_item,
                 created_at_ms)
             SELECT ?1, ?2, ?3, ?4, ?5, NULL, ?6 WHERE ?4 >= 0 AND {} AND {}
             ON CONFLICT(channel_id, source, id, ingredient_index) DO UPDATE SET
                user_id = excluded.user_id,
                pantry_item = NULL,
                created_at = unixepoch(),
                created_at_ms = excluded.created_at_ms",
            seated_in_a_started_plan("?5"),
            not_against_the_decision("?2", "?3")
        ),
        libsql::params![channel, source, id, index, user, at],
    )
    .await?;
    Ok(())
}

/// Put one line back on the shopping list — the tick is the row, so clearing it is a
/// delete. Deleting nothing is success: unticking something already unticked is the
/// state the caller asked for.
///
/// A pantry pre-tick (#156) goes the same way as anyone's, and that is the point: the
/// jar was empty, and saying so must not be a special case. The seed marker in
/// `buy_seeds` is what stops the pantry putting it straight back.
///
/// Clearing is a write on a shared list, so it carries the same predicates the tick
/// does ([`seated_in_a_started_plan`], [`not_against_the_decision`]) and therefore
/// needs to know *who* is asking, even though the row it removes records somebody
/// else. Anyone deciding this meal may put anything back; nobody else may. A guarded
/// claim with an unguarded release is not guarded, and that holds for the recipe as
/// much as for the person.
///
/// **No instant.** The row is the tick, so putting a line back removes it, and there is
/// nothing left to stamp — an untick with a recorded instant would be a tombstone, which
/// is the thing `buy_checks` and `plan_timers` both refuse to hold.
pub(crate) async fn untick_item(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    index: i64,
    user: &str,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "DELETE FROM buy_checks
              WHERE channel_id = ?1 AND source = ?2 AND id = ?3 AND ingredient_index = ?4
                AND {} AND {}",
            seated_in_a_started_plan("?5"),
            not_against_the_decision("?2", "?3")
        ),
        libsql::params![channel, source, id, index, user],
    )
    .await?;
    Ok(())
}

// ---- the pantry seed (#156) ------------------------------------------------

/// Whether this meal's list for this recipe has already been seeded from the pantry.
async fn buy_list_seeded(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM buy_seeds WHERE channel_id = ?1 AND source = ?2 AND id = ?3",
            libsql::params![channel, source, id],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// The shopping list's ingredient names, in the list's own index order, or `None` when
/// the corpus holds no such recipe.
///
/// `None` and `Some(vec![])` are different answers and both matter: no row means there
/// is no list to seed *yet* (so nothing is recorded and the next ask tries again),
/// while a row with no readings means the recipe is unread and genuinely has nothing to
/// match — that list is seeded, with nothing in it.
async fn shopping_names_of(
    conn: &Connection,
    source: &str,
    id: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    let mut rows = conn
        .query(
            "SELECT ingredients FROM recipes WHERE source = ?1 AND id = ?2",
            libsql::params![source, id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let json: String = row.get(0)?;
    // A row whose ingredients no longer parse is treated as a list with no names, the
    // same way `enrich::load` skips a reading that no longer deserializes: the shop
    // still works, it just starts empty.
    let ingredients: Vec<recipe_core::Ingredient> = serde_json::from_str(&json).unwrap_or_default();
    Ok(Some(recipe_core::pantry::shopping_names(&ingredients)))
}

/// The pantry of the kitchen this plan belongs to, indexed for matching. A plan with no
/// kitchen has no pantry, and that is an empty one rather than an error — plans without
/// a kitchen are ordinary (6 of the 9 in production today).
async fn plan_pantry(
    conn: &Connection,
    channel: &str,
) -> anyhow::Result<recipe_core::pantry::Pantry> {
    let mut rows = conn
        .query(
            "SELECT p.item
             FROM pick_sessions s
             JOIN kitchen_pantry p ON p.kitchen_id = s.kitchen_id
             WHERE s.channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let mut items = Vec::new();
    while let Some(r) = rows.next().await? {
        items.push(r.get::<String>(0)?);
    }
    Ok(recipe_core::pantry::Pantry::new(items))
}

/// Write the seed: the marker first, then the pre-ticks.
///
/// **Marker first, deliberately.** If the process dies between the two writes the list
/// is left under-seeded rather than seedable again, and under-seeding is the safe
/// direction throughout this feature — a missed pre-tick costs a jar of salt, a
/// resurrected one costs the dinner. It also makes the write idempotent under the
/// `with_db` retry (#130), which may run this closure more than once.
///
/// `DO NOTHING` on conflict so a person who ticked a line in the same breath keeps it:
/// the seed never overwrites a claim.
///
/// **Both statements carry [`not_against_the_decision`] since #201**, and that does not
/// undo the reasoning that leaves this ungated by [`seated_in_a_started_plan`]. A
/// pre-tick is still nobody's claim and still depends on nothing about the caller; what
/// this predicate asks about is the *plan and the recipe*, which is precisely what the
/// seed is a function of. The one thing a stranger holding the channel id could
/// previously make happen — a list, and a pantry read, for a recipe the plan never
/// agreed on — is the enforcement gap #201 names, so it goes in the write rather than
/// resting on the handlers that now ask the same question a round trip earlier.
///
/// If the decision lands between the marker and the pre-ticks, the marker stands and
/// the pre-ticks are refused: **under**-seeded, the safe direction this whole feature
/// leans in, and for a list nothing will ever read anyway.
///
/// **`at` is the server's own clock** (#209), not an initiator's, and that is the honest
/// answer rather than a shortcut: nobody taps a pre-tick. The seed is something this
/// process does when a list is first asked for, so the moment it happened *is* the
/// moment the process did it — and the server's clock is the shared timeline itself
/// ([`crate::events::server_now_ms`]), so there is no drift to compensate for and no
/// participant whose drift it would be. Borrowing the asking tick's instant would date
/// the kitchen's cupboard by somebody's phone.
async fn write_seed(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    preticks: &[(usize, String)],
    at: i64,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO buy_seeds (channel_id, source, id, seeded_at_ms)
             SELECT ?1, ?2, ?3, ?4 WHERE {}
             ON CONFLICT(channel_id, source, id) DO NOTHING",
            not_against_the_decision("?2", "?3")
        ),
        libsql::params![channel, source, id, at],
    )
    .await?;
    for (index, item) in preticks {
        conn.execute(
            &format!(
                "INSERT INTO buy_checks
                    (channel_id, source, id, ingredient_index, user_id, pantry_item,
                     created_at_ms)
                 SELECT ?1, ?2, ?3, ?4, NULL, ?5, ?6 WHERE {}
                 ON CONFLICT(channel_id, source, id, ingredient_index) DO NOTHING",
                not_against_the_decision("?2", "?3")
            ),
            libsql::params![channel, source, id, *index as i64, item.clone(), at],
        )
        .await?;
    }
    Ok(())
}

/// Seed this meal's list for this recipe from the plan's kitchen pantry, once (#156).
///
/// **When the list is first built.** There is no earlier server-side moment to use: the
/// pick's decision is stashed by the browser (see [`BuyQuery`]), so `(channel, source,
/// id)` first exists here, the first time anyone asks for that list — a read or a
/// write, whichever comes first. A read that writes is unusual and this one is stated
/// rather than hidden. It is safe to reach from the ungated read ([`buy_list`]) because
/// the seed depends on nothing about the caller: it is a function of the plan's kitchen
/// and the recipe, so a stranger holding the channel id can at most make it happen
/// slightly sooner than the first member would have.
///
/// **Once, and recorded.** `buy_seeds` holds the fact that it ran, so stock added to
/// the kitchen mid-shop does not re-tick, and — the case that actually bites —
/// unticking the last pre-tick and reloading does not put it all back. See the
/// migration for why that is a table and not a heuristic.
/// **One connection, both ways in.** The seed is reached from the ungated read
/// ([`buy_list`]) and from a shopping tick's own handler
/// ([`crate::events::SessionEvent::BuyTick`]), and since #209 the second of those runs
/// inside the event framework, which holds a plain [`Connection`] rather than the
/// [`AppState`] an HTTP handler has. So the whole sequence lives here on a connection,
/// and [`ensure_buy_seed`] is the one-line `with_db` wrapper for the read that still
/// has one.
///
/// Retrying the whole sequence is safe, which is why it can be one `with_db` closure
/// instead of four: the marker is `DO NOTHING` on conflict and the pre-ticks are too, so
/// a second run of a partially-completed seed finishes it rather than doubling it (#130).
pub(crate) async fn seed_buy_list(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<()> {
    if buy_list_seeded(conn, channel, source, id).await? {
        return Ok(());
    }
    let pantry = plan_pantry(conn, channel).await?;
    // No such recipe: there is no list yet, so nothing was created and nothing is
    // recorded. The next ask — after a derive, say — gets its seed.
    let Some(names) = shopping_names_of(conn, source, id).await? else {
        return Ok(());
    };
    let preticks = recipe_core::pantry::preticks(&names, &pantry);
    write_seed(
        conn,
        channel,
        source,
        id,
        &preticks,
        crate::events::server_now_ms(),
    )
    .await
}

async fn ensure_buy_seed(
    state: &AppState,
    channel: &str,
    source: &str,
    id: &str,
) -> Result<(), AppError> {
    state
        .with_db(move |db| async move { seed_buy_list(&db, channel, source, id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// One recipe's checklist in a meal: the ticked lines, in ingredient order, each with
/// where its tick came from. The `users` join is a LEFT one for the same reason the
/// lobby's is — a handle is a display convenience and may be absent, and now also
/// because a pantry pre-tick has no user row to join to at all.
pub(crate) async fn load_buy_checks(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<Vec<BuyCheck>> {
    let mut rows = conn
        .query(
            "SELECT b.ingredient_index, b.user_id, u.username, b.pantry_item
             FROM buy_checks b
             LEFT JOIN users u ON u.telegram_user_id = b.user_id
             WHERE b.channel_id = ?1 AND b.source = ?2 AND b.id = ?3
             ORDER BY b.ingredient_index",
            libsql::params![channel, source, id],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        // The CHECK in migration 0021 makes these mutually exclusive, so the row is
        // read as it is rather than one being preferred over the other.
        let user_id: Option<String> = r.get(1)?;
        let username: Option<String> = r.get(2)?;
        out.push(BuyCheck {
            index: r.get(0)?,
            by: user_id.map(|telegram_user_id| Voter {
                telegram_user_id,
                username,
            }),
            pantry: r.get(3)?,
        });
    }
    Ok(out)
}

/// The tally for a channel: distinct-voter count plus per-recipe yes/no, ranked by
/// yeses. Plurality (rank by `yes`) is derived from this alone; consensus is
/// `yes == deciders && no == 0`, and `deciders` is the **roster** on
/// [`ServerMsg::Lobby`], not the voter count returned here (#181).
async fn load_tally(conn: &Connection, channel: &str) -> anyhow::Result<(i64, Vec<TallyRow>)> {
    let mut prows = conn
        .query(
            "SELECT COUNT(DISTINCT voter_id) FROM votes WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let participants: i64 = match prows.next().await? {
        Some(r) => r.get(0)?,
        None => 0,
    };

    // Who said yes to what, in the order they said it — read as its own pass rather
    // than folded into the aggregate below, so the grouped query stays the plain
    // ranking it has always been and this stays a list of ids rather than a string
    // the caller has to take apart.
    let mut yrows = conn
        .query(
            "SELECT source, id, voter_id FROM votes
             WHERE channel_id = ?1 AND vote = 1
             ORDER BY created_at, voter_id",
            libsql::params![channel],
        )
        .await?;
    let mut yes_by_recipe: HashMap<(String, String), Vec<String>> = HashMap::new();
    while let Some(r) = yrows.next().await? {
        yes_by_recipe
            .entry((r.get(0)?, r.get(1)?))
            .or_default()
            .push(r.get(2)?);
    }

    let mut rows = conn
        .query(
            "SELECT source, id,
                    SUM(CASE WHEN vote = 1 THEN 1 ELSE 0 END) AS yes,
                    SUM(CASE WHEN vote = 0 THEN 1 ELSE 0 END) AS no
             FROM votes WHERE channel_id = ?1
             GROUP BY source, id
             ORDER BY yes DESC, no ASC",
            libsql::params![channel],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let source: String = r.get(0)?;
        let id: String = r.get(1)?;
        let yes_voters = yes_by_recipe
            .remove(&(source.clone(), id.clone()))
            .unwrap_or_default();
        out.push(TallyRow {
            source,
            id,
            yes: r.get(2)?,
            no: r.get(3)?,
            yes_voters,
        });
    }
    Ok((participants, out))
}

/// Plan fixtures shared with the modules that write to a plan from outside this one —
/// [`crate::timers`] through the event framework, today.
///
/// They live here because the states they build are this module's (`create_session`,
/// `seat_voter`, `begin_session` and the decision are all private), and because a
/// second module hand-rolling "a started plan" out of raw INSERTs would be building a
/// state no client can produce — which is exactly the mistake the docstring below
/// records.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A plan mid-swipe: `who` on the roster and the lobby closed behind them — the
    /// only state a vote or a shopping claim can be written in (#175). Fixtures that
    /// wrote either against a bare `create_session` were building a state no client
    /// can produce, which is exactly what let the gap sit unnoticed.
    pub(crate) async fn started_plan(conn: &Connection, channel: &str, who: &[&str]) {
        lobby(conn, channel, who).await;
        begin_session(conn, channel).await.unwrap();
    }

    /// The same plan with its lobby still open — nothing shared may be written yet.
    pub(crate) async fn lobby(conn: &Connection, channel: &str, who: &[&str]) {
        create_session(
            conn,
            channel,
            who.first().copied().unwrap_or("alice"),
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        for person in who {
            seat_voter(conn, channel, person).await.unwrap();
        }
    }

    /// Record what the plan decided (#201), the way the deciding vote's write does —
    /// all four columns together, including the instant the deciding swipe happened at
    /// (#209), so a fixture cannot produce a row `decide_if_agreed` never could.
    pub(crate) async fn decide(conn: &Connection, channel: &str, source: &str, id: &str) {
        conn.execute(
            "UPDATE pick_sessions
                SET decided_source = ?2, decided_id = ?3, decided_at = unixepoch(),
                    decided_at_ms = ?4
              WHERE channel_id = ?1",
            libsql::params![channel, source, id, 1_700_000_000_000i64],
        )
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{decide, started_plan};
    use super::*;

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    /// **The instant an event happened**, as the framework hands it to a handler
    /// (#209): already normalised into the shared timeline by `events::normalize`, in
    /// unix milliseconds.
    ///
    /// One named constant rather than a literal per call, because almost every test
    /// below is about *what* a write does and not *when* — the ones that are about the
    /// when say so by passing their own instant. Nothing in these handlers reads a
    /// clock, so this is a plain input like the channel id beside it, and a stated one
    /// beats `unixepoch()`-at-test-time for the same reason it beats it in production.
    const TAP: i64 = 1_700_000_000_000;

    fn row<'a>(rows: &'a [TallyRow], id: &str) -> &'a TallyRow {
        rows.iter().find(|r| r.id == id).expect("a tally row")
    }

    /// Two voters, two recipes: the tally counts yes/no per recipe and the distinct
    /// voters, and ranks by yeses — enough for the client to read both plurality and
    /// consensus off it.
    /// **Every plan is born with a seed** (#212), and no two plans share one.
    ///
    /// The seed is the whole feature: a room's soundtrack is a pure function of it and
    /// of how long the plan has existed, so nothing is stored, announced or raced over.
    /// A plan created without one has no shared randomness at all, which is why this is
    /// asserted on `create_session` rather than left to a column default — SQLite cannot
    /// require it, so the writer must.
    #[tokio::test]
    async fn every_plan_is_born_with_its_own_seed() {
        let conn = conn().await;
        let mut seen = std::collections::BTreeSet::new();
        for channel in ["c", "d", "e", "f"] {
            create_session(
                &conn,
                channel,
                "alice",
                None,
                None,
                MealType::Dinner,
                &[],
                None,
            )
            .await
            .unwrap();
            let view = load_lobby(&conn, channel).await.unwrap().unwrap();
            let seed = view.seed.expect("a new plan has a seed");
            assert!(
                (0..(1i64 << 53)).contains(&seed),
                "a seed must survive JSON into a browser's Number exactly: {seed}"
            );
            assert!(seen.insert(seed), "two plans dealt the same seed: {seed}");
        }
    }

    /// **And the birth instant beside it** — the anchor everything derived from the seed
    /// is measured from. The pair travels together on one read of one row, so a plan can
    /// never be described by one plan's seed and another's clock.
    #[tokio::test]
    async fn the_lobby_carries_the_seed_and_the_plans_birth_instant() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert!(view.seed.is_some());
        // `created_at` defaults to `unixepoch()` (0006), so it is a real second in the
        // present rather than a zero nobody set.
        assert!(
            view.created_at > 1_700_000_000,
            "a plan is born now, not in 1970: {}",
            view.created_at
        );
    }

    /// **A plan from before plans had seeds says so.** Migration 0031 is additive and
    /// backfills nothing, because inventing a seed for a plan now would be inventing a
    /// shared past its participants never had — and a *default* would be worse, putting
    /// every such plan on one station. `None` reads as "no shared randomness", and the
    /// surface that consumes it falls back to each device's own (#146: degrade, do not
    /// die).
    #[tokio::test]
    async fn a_plan_from_before_the_seed_has_none() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        conn.execute(
            "UPDATE pick_sessions SET seed = NULL WHERE channel_id = ?1",
            libsql::params!["c"],
        )
        .await
        .unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.seed, None, "absent, not zero");
        assert!(view.created_at > 0, "and the anchor is still a real instant");
    }

    /// **The lobby frame carries the two facts and nothing about anybody's speaker.**
    ///
    /// The on/off switch is personal: sync decides *what* plays and *where in it we are*,
    /// never whether a given device makes a sound. There is no music frame at all any
    /// more, and this one has nowhere to say so — asserted as the **whole key set**,
    /// because a field nobody thought to look for is exactly what this has to catch.
    #[test]
    fn the_lobby_frame_says_nothing_about_whether_a_device_is_audible() {
        let wire = serde_json::to_string(&ServerMsg::Lobby {
            deciders: 2,
            started: true,
            seed: Some(4242),
            created_at: 1_700_000_000,
        })
        .unwrap();
        let keys: std::collections::BTreeSet<String> =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&wire)
                .unwrap()
                .keys()
                .cloned()
                .collect();
        assert_eq!(
            keys,
            ["created_at", "deciders", "seed", "started", "type"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        );

        // A plan with no seed is `null` on the wire, not missing and not 0.
        let none = serde_json::to_string(&ServerMsg::Lobby {
            deciders: 1,
            started: false,
            seed: None,
            created_at: 1_600_000_000,
        })
        .unwrap();
        assert!(none.contains(r#""seed":null"#), "{none}");
    }

    /// A plan is never roster-less and never double-counts: the host is seated once,
    /// however many times they arrive.
    #[tokio::test]
    async fn seating_is_idempotent_and_the_lobby_reads_back() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            Some("k1"),
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        seat_voter(&conn, "c", "bob").await.unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.host, "alice");
        assert_eq!(view.kitchen_id.as_deref(), Some("k1"));
        assert!(!view.started, "a fresh plan is still in its lobby");
        assert_eq!(view.voters.len(), 2, "alice once, plus bob");
        assert_eq!(view.voters[0].telegram_user_id, "alice");
    }

    /// Voters come back with their usernames, which means reading the far column of
    /// the row rather than merely counting rows — the shape that hid a column-index
    /// slip in the kitchens lobby's twin query.
    #[tokio::test]
    async fn voters_carry_their_usernames() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)",
            libsql::params!["4242", "dave"],
        )
        .await
        .unwrap();
        create_session(&conn, "c", "4242", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "4242").await.unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.voters.len(), 1);
        assert_eq!(view.voters[0].telegram_user_id, "4242");
        assert_eq!(view.voters[0].username.as_deref(), Some("dave"));
    }

    /// A plan in a kitchen offers that kitchen's members as candidates, minus whoever
    /// is already deciding — the pool the host can pull in without a link (#72).
    #[tokio::test]
    async fn a_kitchen_plan_offers_its_members_as_candidates() {
        let conn = conn().await;
        let kid = crate::kitchens::create_kitchen(&conn, "Home", "host")
            .await
            .unwrap();
        crate::kitchens::seat_member_for_test(&conn, &kid, "mel").await;
        crate::kitchens::seat_member_for_test(&conn, &kid, "sam").await;

        create_session(
            &conn,
            "c",
            "host",
            None,
            Some(&kid),
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        seat_voter(&conn, "c", "host").await.unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        let candidates: Vec<&str> = view
            .candidates
            .iter()
            .map(|v| v.telegram_user_id.as_str())
            .collect();
        assert!(candidates.contains(&"mel"), "{candidates:?}");
        assert!(candidates.contains(&"sam"), "{candidates:?}");
        assert!(
            !candidates.contains(&"host"),
            "the host is deciding, not a candidate"
        );
    }

    /// A plan with no kitchen has no candidate pool — it is invite-only.
    #[tokio::test]
    async fn a_kitchenless_plan_has_no_candidates() {
        let conn = conn().await;
        create_session(&conn, "c", "host", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "host").await.unwrap();
        assert!(load_lobby(&conn, "c")
            .await
            .unwrap()
            .unwrap()
            .candidates
            .is_empty());
    }

    /// Starting twice must not move the moment the roster closed — a second press is
    /// a no-op, not a re-start.
    #[tokio::test]
    async fn starting_is_idempotent() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        begin_session(&conn, "c").await.unwrap();
        let first: Option<i64> = {
            let mut rows = conn
                .query(
                    "SELECT started_at FROM pick_sessions WHERE channel_id = ?1",
                    libsql::params!["c"],
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };
        begin_session(&conn, "c").await.unwrap();
        let second: Option<i64> = {
            let mut rows = conn
                .query(
                    "SELECT started_at FROM pick_sessions WHERE channel_id = ?1",
                    libsql::params!["c"],
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };
        assert!(first.is_some());
        assert_eq!(first, second, "the start time is the first one, always");
        assert!(load_lobby(&conn, "c").await.unwrap().unwrap().started);
    }

    /// A plan that does not exist has no lobby — and must not be conjured into one.
    #[tokio::test]
    async fn an_unknown_plan_has_no_lobby() {
        let conn = conn().await;
        assert!(load_lobby(&conn, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_vote_and_tally() {
        let conn = conn().await;
        started_plan(&conn, "chan1", &["alice", "bob"]).await;

        record_vote(&conn, "chan1", "themealdb", "r1", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "chan1", "themealdb", "r1", "bob", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "chan1", "themealdb", "r2", "alice", false, TAP)
            .await
            .unwrap();

        let (participants, rows) = load_tally(&conn, "chan1").await.unwrap();
        assert_eq!(participants, 2, "alice + bob");
        // r1 is the consensus/plurality winner: 2 yes, 0 no, == participants.
        assert_eq!((row(&rows, "r1").yes, row(&rows, "r1").no), (2, 0));
        assert_eq!((row(&rows, "r2").yes, row(&rows, "r2").no), (0, 1));
        assert_eq!(rows[0].id, "r1", "ranked by yeses");
    }

    /// A swipe is a current call, not an append: re-voting overwrites the row, so
    /// the tally never double-counts one person.
    #[tokio::test]
    async fn re_voting_updates_not_appends() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        record_vote(&conn, "c", "s", "1", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "s", "1", "alice", false, TAP)
            .await
            .unwrap();

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 1);
        assert_eq!(rows.len(), 1, "one row, not two");
        assert_eq!((rows[0].yes, rows[0].no), (0, 1), "the changed-to no");
    }

    // ---- who may write a vote (#175) ---------------------------------------

    /// The wrong answer this closes, and it needed no crafted request to reach.
    ///
    /// Two people are deciding. A third holds the invite link and opened it after the
    /// swiping began: `join_lobby` refuses them (the roster is closed), and nothing
    /// then stopped them voting — the socket upgrade asks only that the channel
    /// exist, and the insert asked nothing at all. Meanwhile the client reads
    /// consensus as `yes === deciders`, and `deciders` is the **roster**. So one
    /// member's yes plus one outsider's yes read as "everybody agreed", and the plan
    /// jumped to `buy` on a recipe a decider had never seen.
    ///
    /// It also restores what #169 claimed was already structural: a tally can never
    /// carry more yeses than there are deciders.
    #[tokio::test]
    async fn a_yes_from_outside_the_roster_never_reaches_the_tally() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        let deciders = roster(&conn, "c").await.len();

        assert!(record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap());
        assert!(
            !record_vote(&conn, "c", "t", "r1", "mallory", true, TAP)
                .await
                .unwrap(),
            "a signed-in stranger holding the channel id is not deciding this meal"
        );

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        let r1 = row(&rows, "r1");
        assert_eq!((r1.yes, r1.no), (1, 0), "alice's yes, and only alice's");
        assert_eq!(r1.yes_voters, vec!["alice".to_owned()]);
        assert!(
            (r1.yes as usize) < deciders,
            "bob has not swiped, so this is not agreement"
        );
        assert_eq!(participants, 1, "one person has voted, not two");
    }

    /// "Votes only exist after the start" is the premise #169 rests its absent sweeps
    /// on, and until now the browser was the only thing holding it up: the vote path
    /// has no preceding read to carry it, so the insert carries it itself.
    ///
    /// Not a race — `started_at` only goes NULL → set, so nobody can lose one. It is
    /// the invariant, asserted where it is claimed.
    #[tokio::test]
    async fn a_vote_before_the_start_is_not_a_vote() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();

        assert!(
            !record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
                .await
                .unwrap(),
            "the lobby is where you gather, not where you swipe"
        );
        assert_eq!(
            load_tally(&conn, "c").await.unwrap().0,
            0,
            "and the refusal is real: nothing was written"
        );

        begin_session(&conn, "c").await.unwrap();
        assert!(record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap());
        assert_eq!(load_tally(&conn, "c").await.unwrap().0, 1);
    }

    /// A channel nobody ever created is not a room to vote into, the same answer the
    /// WS upgrade and the walk already give a mistyped channel — and now the answer
    /// holds even for a socket that was opened before the plan was emptied.
    #[tokio::test]
    async fn a_vote_into_a_plan_that_is_gone_is_not_recorded() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert!(record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap());

        conn.execute("DELETE FROM pick_sessions WHERE channel_id = 'c'", ())
            .await
            .unwrap();
        assert!(
            !record_vote(&conn, "c", "t", "r2", "alice", true, TAP)
                .await
                .unwrap(),
            "no plan, no vote"
        );
        assert!(!load_tally(&conn, "c")
            .await
            .unwrap()
            .1
            .iter()
            .any(|r| r.id == "r2"));
    }

    // ---- the decision is a server fact (#201) -------------------------------

    /// Everyone on the roster said yes to one recipe and nobody said no — the win
    /// condition, and the only thing that ends a pick.
    ///
    /// The deciding count is the **roster**, which is what "everyone needs to be based
    /// on the lobby" means when the server is the one saying it: bob's yes is the third
    /// of three and it decides, and it would have decided on nothing before it. The
    /// recipe alice alone liked never comes near it, however many yeses it has relative
    /// to who has swiped.
    #[tokio::test]
    async fn the_last_yes_from_the_whole_roster_decides_the_plan() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob", "carol"]).await;

        for who in ["alice", "carol"] {
            record_vote(&conn, "c", "t", "r1", who, true, TAP)
                .await
                .unwrap();
            assert!(
                decide_if_agreed(&conn, "c", "t", "r1", TAP)
                    .await
                    .unwrap()
                    .is_none(),
                "{who} agreeing is not everyone agreeing"
            );
        }
        // The recipe only alice wants, so the tally holds something that must never be
        // mistaken for agreement.
        record_vote(&conn, "c", "t", "r2", "alice", true, TAP)
            .await
            .unwrap();
        assert!(decide_if_agreed(&conn, "c", "t", "r2", TAP)
            .await
            .unwrap()
            .is_none());

        record_vote(&conn, "c", "t", "r1", "bob", true, TAP)
            .await
            .unwrap();
        let decided = decide_if_agreed(&conn, "c", "t", "r1", TAP)
            .await
            .unwrap()
            .expect("the third yes of three decides");
        assert_eq!((decided.source.as_str(), decided.id.as_str()), ("t", "r1"));
        assert!(decided.decided_at > 0, "and it is stamped, not merely true");
        assert_eq!(
            load_decision(&conn, "c").await.unwrap(),
            Some(decided),
            "recorded on the plan, not returned and forgotten"
        );
    }

    /// One short of the roster is not agreement, and the gap is exactly the bug the
    /// distinct-voter count produces: two of three have said yes, so `yes ==
    /// participants` is true and `yes == deciders` is not. The roster is what is asked.
    #[tokio::test]
    async fn a_roster_one_yes_short_decides_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob", "carol"]).await;
        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "bob", true, TAP)
            .await
            .unwrap();

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(
            (participants, row(&rows, "r1").yes),
            (2, 2),
            "unanimous by turnout, which is the number that must not decide"
        );
        assert!(
            decide_if_agreed(&conn, "c", "t", "r1", TAP)
                .await
                .unwrap()
                .is_none(),
            "carol has not swiped it, and not swiping is not agreeing"
        );
        assert_eq!(load_decision(&conn, "c").await.unwrap(), None);
    }

    /// One no is a veto, however full the house. Carol's pass holds back a recipe alice
    /// and bob both wanted — checked as its own clause rather than by subtracting from
    /// the yes count, because a person's no is a row, not the absence of their yes.
    #[tokio::test]
    async fn one_no_holds_back_a_recipe_everyone_else_wanted() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob", "carol"]).await;
        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "bob", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "carol", false, TAP)
            .await
            .unwrap();

        assert!(
            decide_if_agreed(&conn, "c", "t", "r1", TAP)
                .await
                .unwrap()
                .is_none(),
            "two out of three is a majority and a pick is not a vote"
        );

        // And a change of heart is a change of heart: the same swipe the other way
        // decides, because a vote is a current call rather than an append.
        record_vote(&conn, "c", "t", "r1", "carol", true, TAP)
            .await
            .unwrap();
        assert!(decide_if_agreed(&conn, "c", "t", "r1", TAP)
            .await
            .unwrap()
            .is_some());
    }

    /// Why the veto clause is redundant, asserted rather than assumed.
    ///
    /// `votes` is keyed `(channel_id, source, id, voter_id)`, so one person holds one
    /// row per recipe: they are a yes or a no on it, never both. That is the whole
    /// reason "every decider said yes" already implies "no decider said no", and so the
    /// reason a mutation dropping `decide_if_agreed`'s `NOT EXISTS (… vote = 0)` clause
    /// survives every test — it is an equivalent mutation, not an untested one.
    ///
    /// The redundancy is the interesting thing, so the fact underneath it is pinned
    /// here. If this key ever widened — an append-only vote log, a per-round key — the
    /// clause stops being redundant and starts being the only thing standing between a
    /// veto and a decision, which is exactly when nobody would think to add it.
    #[tokio::test]
    async fn a_persons_vote_is_one_row_so_a_yes_and_a_no_cannot_coexist() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        // Written straight in, because the API cannot ask for this: `record_vote`'s
        // upsert would overwrite the yes rather than sit beside it.
        let refused = conn
            .execute(
                "INSERT INTO votes (channel_id, source, id, voter_id, vote)
                 VALUES ('c', 't', 'r1', 'alice', 0)",
                (),
            )
            .await;
        assert!(
            refused.is_err(),
            "a second row for one person on one recipe is not a state this table has"
        );

        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(
            (row(&rows, "r1").yes, row(&rows, "r1").no),
            (1, 0),
            "so a full house of yeses can never have a no hiding under it"
        );
    }

    /// **First past the post.** Two recipes reach the condition, and only one is ever
    /// recorded.
    ///
    /// Both calls run the same UPDATE and the guard `decided_at IS NULL` is inside it,
    /// so the second is judged against the row the first left behind rather than
    /// against the read that preceded it — which is the state two simultaneously
    /// completing votes produce. Drop that clause and the second call overwrites the
    /// first: this test then reports r2 where the room was told r1, which is a group
    /// standing in a shop holding two different lists.
    #[tokio::test]
    async fn only_the_first_completing_vote_records_a_decision() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        for recipe in ["r1", "r2"] {
            for who in ["alice", "bob"] {
                record_vote(&conn, "c", "t", recipe, who, true, TAP)
                    .await
                    .unwrap();
            }
        }

        let first = decide_if_agreed(&conn, "c", "t", "r1", TAP)
            .await
            .unwrap()
            .expect("r1 met the condition");
        assert!(
            decide_if_agreed(&conn, "c", "t", "r2", TAP)
                .await
                .unwrap()
                .is_none(),
            "r2 meets it too, and the plan has already decided — the loser says nothing"
        );
        assert_eq!(
            load_decision(&conn, "c").await.unwrap(),
            Some(first),
            "the first record stands, timestamp and all"
        );
    }

    /// A decided plan's deck is over, so a swipe arriving after it is **refused**, not
    /// counted — the guard is in the insert's own predicate, so it holds for a socket
    /// that read "undecided" a round trip ago.
    ///
    /// Refused rather than ignored: `record_vote` answers `false`, the room is told
    /// nothing, and the tally is untouched. The client is not left guessing either — it
    /// has already been sent the decision, or will be the moment it connects.
    #[tokio::test]
    async fn a_vote_after_the_decision_is_not_recorded() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "bob", true, TAP)
            .await
            .unwrap();
        decide_if_agreed(&conn, "c", "t", "r1", TAP).await.unwrap();
        let (before_participants, before) = load_tally(&conn, "c").await.unwrap();

        assert!(
            !record_vote(&conn, "c", "t", "r2", "alice", true, TAP)
                .await
                .unwrap(),
            "a new recipe cannot be swiped into a plan that has finished"
        );
        assert!(
            !record_vote(&conn, "c", "t", "r1", "bob", false, TAP)
                .await
                .unwrap(),
            "and neither can the decided one be taken back"
        );

        let (after_participants, after) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(before_participants, after_participants);
        assert_eq!(
            before.len(),
            after.len(),
            "nothing was written, so there is no r2 row"
        );
        assert_eq!(
            (row(&after, "r1").yes, row(&after, "r1").no),
            (2, 0),
            "and bob's yes still stands — the refusal did not overwrite it"
        );
    }

    /// The record is not recomputable-away. Once written it stands, even against a
    /// tally that no longer supports it.
    ///
    /// The votes should never move — nothing can write one after the decision, which
    /// the test above pins — but the whole point of recording the fact rather than
    /// deriving it is that "what we decided" survives whatever the rows do. Deleting
    /// them directly is the state no API path can reach, asserted where it is claimed.
    #[tokio::test]
    async fn the_decision_outlives_the_votes_that_made_it() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        for who in ["alice", "bob"] {
            record_vote(&conn, "c", "t", "r1", who, true, TAP)
                .await
                .unwrap();
        }
        let decided = decide_if_agreed(&conn, "c", "t", "r1", TAP).await.unwrap();
        assert!(decided.is_some());

        conn.execute("DELETE FROM votes WHERE channel_id = 'c'", ())
            .await
            .unwrap();
        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(
            (participants, rows.len()),
            (0, 0),
            "the tally now says nothing was ever agreed"
        );
        assert_eq!(
            load_decision(&conn, "c").await.unwrap(),
            decided,
            "and the plan still says what it decided"
        );
    }

    /// A lobby decides nothing. No vote can exist before the start (#175/#179), so the
    /// clause can only ever fire on a plan whose votes predate the guard — and it is
    /// asked anyway, because a guard that holds only while a *different* guard holds is
    /// the kind that quietly stops holding.
    #[tokio::test]
    async fn a_plan_that_has_not_started_decides_nothing() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        // Written straight in: `record_vote` would refuse this, which is the point —
        // the two guards make each other's gap unreachable, and each still holds alone.
        conn.execute(
            "INSERT INTO votes (channel_id, source, id, voter_id, vote)
             VALUES ('c', 't', 'r1', 'alice', 1)",
            (),
        )
        .await
        .unwrap();

        assert!(
            decide_if_agreed(&conn, "c", "t", "r1", TAP)
                .await
                .unwrap()
                .is_none(),
            "the lobby is where you gather, not where anything is decided"
        );

        begin_session(&conn, "c").await.unwrap();
        assert!(
            decide_if_agreed(&conn, "c", "t", "r1", TAP)
                .await
                .unwrap()
                .is_some(),
            "and the same votes decide once the swiping has begun"
        );
    }

    /// A roster of nobody agrees to nothing — arithmetic, not paranoia. With the
    /// `EXISTS (roster)` clause dropped the counts read `0 = 0`, so **every** recipe
    /// nobody voted against would be decided, and the first buy request would name one.
    #[tokio::test]
    async fn an_empty_roster_decides_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        conn.execute("DELETE FROM pick_voters WHERE channel_id = 'c'", ())
            .await
            .unwrap();

        assert!(
            decide_if_agreed(&conn, "c", "t", "never-swiped", TAP)
                .await
                .unwrap()
                .is_none(),
            "a recipe with no votes at all is not what a plan with no deciders agreed"
        );
        assert_eq!(load_decision(&conn, "c").await.unwrap(), None);
    }

    /// Both counts are taken over the roster, so a vote from outside it can neither
    /// complete a consensus nor veto one.
    ///
    /// #179 already keeps such a row out of `votes`, so these are written directly —
    /// the state a row predating that guard would be in. The ruling is that the roster
    /// is the deciding body, and it is said once in each direction: mallory's yes does
    /// not stand in for bob, and mallory's no does not overrule him.
    #[tokio::test]
    async fn a_vote_from_outside_the_roster_neither_completes_nor_vetoes() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO votes (channel_id, source, id, voter_id, vote)
             VALUES ('c', 't', 'r1', 'mallory', 1)",
            (),
        )
        .await
        .unwrap();

        assert!(
            decide_if_agreed(&conn, "c", "t", "r1", TAP)
                .await
                .unwrap()
                .is_none(),
            "a stranger's yes is not bob's"
        );

        conn.execute("UPDATE votes SET vote = 0 WHERE voter_id = 'mallory'", ())
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "bob", true, TAP)
            .await
            .unwrap();
        assert!(
            decide_if_agreed(&conn, "c", "t", "r1", TAP)
                .await
                .unwrap()
                .is_some(),
            "and a stranger's no does not overrule the people deciding"
        );
    }

    /// The lobby read carries the decision, so the one HTTP answer that describes a
    /// whole plan can say the plan is over — and says it from the same row, so it can
    /// never disagree with the frame.
    #[tokio::test]
    async fn the_lobby_carries_the_decision() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert_eq!(
            load_lobby(&conn, "c").await.unwrap().unwrap().decided,
            None,
            "a running plan has decided nothing, and says so rather than omitting it"
        );

        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        let decided = decide_if_agreed(&conn, "c", "t", "r1", TAP).await.unwrap();
        assert_eq!(
            load_lobby(&conn, "c").await.unwrap().unwrap().decided,
            decided
        );
    }

    /// Half a decision is corruption, and it fails loud rather than serving a recipe
    /// with no time on it or a time with no recipe.
    ///
    /// One statement writes all three columns or none, so this state has no producer —
    /// which is exactly why it is pinned here. SQLite cannot add a CHECK with `ALTER
    /// TABLE ADD COLUMN` (migration 0026), so the invariant is asserted on the way out,
    /// the same ruling `load_lobby` gives a `meal_type` outside the vocabulary.
    #[tokio::test]
    async fn half_a_decision_is_refused_loudly() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        conn.execute(
            "UPDATE pick_sessions SET decided_source = 't' WHERE channel_id = 'c'",
            (),
        )
        .await
        .unwrap();

        let err = load_decision(&conn, "c").await.unwrap_err().to_string();
        assert!(
            err.contains("half a decision"),
            "a wrong database must not run beautifully: {err}"
        );
        assert!(
            load_lobby(&conn, "c").await.is_err(),
            "and the lobby that carries it refuses too"
        );
    }

    /// A shopping write for a recipe the plan did not decide is refused **in the
    /// write**, not merely in the handler's read.
    ///
    /// This is the enforcement gap #201 names: `BuyQuery` took the recipe on trust, so
    /// any seated member could name any `(source, id)` and fill a basket for a dinner
    /// nobody agreed to. The predicate here is what makes the refusal true whenever the
    /// decision lands after whatever read the caller did — and since #209 a shopping
    /// write has no preceding read of its own at all, so this is the only thing
    /// standing between a client's `(source, id)` and the row.
    #[tokio::test]
    async fn a_shopping_write_is_judged_against_the_decided_recipe() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        // Undecided: the list still admits any recipe, which is what keeps a decision
        // stashed in a browser before #201 deployed working (0026 backfills nothing).
        tick_item(&conn, "c", "t", "r9", 0, "alice", TAP)
            .await
            .unwrap();
        assert_eq!(
            load_buy_checks(&conn, "c", "t", "r9").await.unwrap().len(),
            1
        );

        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        decide_if_agreed(&conn, "c", "t", "r1", TAP).await.unwrap();

        tick_item(&conn, "c", "t", "r9", 1, "alice", TAP)
            .await
            .unwrap();
        assert_eq!(
            load_buy_checks(&conn, "c", "t", "r9").await.unwrap().len(),
            1,
            "the plan decided r1, so nothing more goes on r9's list"
        );
        untick_item(&conn, "c", "t", "r9", 0, "alice")
            .await
            .unwrap();
        assert_eq!(
            load_buy_checks(&conn, "c", "t", "r9").await.unwrap().len(),
            1,
            "and a guarded claim with an unguarded release is not guarded"
        );

        tick_item(&conn, "c", "t", "r1", 0, "alice", TAP)
            .await
            .unwrap();
        assert_eq!(
            load_buy_checks(&conn, "c", "t", "r1").await.unwrap().len(),
            1,
            "the decided meal shops exactly as it always did"
        );
    }

    /// A channel with no votes yet tallies to nothing — the join rehydrate on a
    /// brand-new session is empty, not an error.
    #[tokio::test]
    async fn empty_channel_tallies_to_nothing() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 0);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn session_existence_gates_join() {
        let conn = conn().await;
        assert!(!session_exists(&conn, "nope").await.unwrap());
        create_session(
            &conn,
            "yep",
            "alice",
            Some(r#"{"area":"Japanese"}"#),
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(session_exists(&conn, "yep").await.unwrap());
    }

    /// A plan is for a meal (#114): created for breakfast, its lobby says breakfast —
    /// stored and read back through the typed vocabulary, never a raw string.
    #[tokio::test]
    async fn a_plan_carries_its_meal_type() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            None,
            MealType::Breakfast,
            &[],
            None,
        )
        .await
        .unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.meal_type, MealType::Breakfast);
    }

    /// An unstated choice is dinner, twice over and identically: the create handler
    /// fills `None` with [`DEFAULT_MEAL_TYPE`], and migration 0016 backfills rows
    /// that predate the column with the same word — so a pre-#114 plan and a
    /// caller who named nothing read the same.
    #[tokio::test]
    async fn an_unstated_meal_type_is_dinner() {
        assert_eq!(DEFAULT_MEAL_TYPE, MealType::Dinner);

        // A body naming no meal deserializes, and resolves to the default —
        // exactly what the create handler does with it.
        let body: CreateBody = serde_json::from_str("{}").unwrap();
        assert_eq!(
            body.meal_type.unwrap_or(DEFAULT_MEAL_TYPE),
            MealType::Dinner
        );

        // A row written without the columns — the shape of every plan that existed
        // before migration 0016 — reads back as a plain dinner via the column
        // defaults: 'dinner', with nothing alongside.
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by) VALUES ('old', 'alice')",
            (),
        )
        .await
        .unwrap();
        let view = load_lobby(&conn, "old").await.unwrap().unwrap();
        assert_eq!(view.meal_type, MealType::Dinner);
        assert!(view.additions.is_empty(), "a plain meal, nothing alongside");
    }

    /// The host repoints the plan at a different meal; the lobby follows.
    #[tokio::test]
    async fn the_meal_type_can_change_while_the_lobby_is_open() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        update_meal_type(&conn, "c", MealType::Lunch).await.unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.meal_type, MealType::Lunch);
    }

    /// The whole vocabulary survives the trip through storage: what `as_str` writes,
    /// `parse` reads, and the wire form serde emits is the same lowercase word — one
    /// canonical spelling everywhere, no second list to drift.
    #[test]
    fn the_meal_type_vocabulary_round_trips() {
        for t in [
            MealType::Breakfast,
            MealType::Lunch,
            MealType::Dinner,
            MealType::Snack,
        ] {
            assert_eq!(MealType::parse(t.as_str()), Some(t), "{t:?}");
            assert_eq!(
                serde_json::to_value(t).unwrap(),
                serde_json::Value::String(t.as_str().to_owned()),
                "the wire form and the stored form are the same word"
            );
        }
    }

    /// The two tiers partition the whole vocabulary: every word belongs to exactly
    /// one of [`MealType`] and [`MealAddition`], and each tier's serde refuses the
    /// other tier's words. This is the classification made checkable — "dessert is
    /// not a meal" is a type error, not a handler's opinion.
    #[test]
    fn the_two_tiers_partition_the_vocabulary() {
        let primary = ["breakfast", "lunch", "dinner", "snack"];
        let secondary = ["dessert", "side"];
        for word in primary {
            let q = format!("{word:?}");
            assert!(serde_json::from_str::<MealType>(&q).is_ok(), "{word}");
            assert!(
                serde_json::from_str::<MealAddition>(&q).is_err(),
                "{word} is a meal, not an addition"
            );
        }
        for word in secondary {
            let q = format!("{word:?}");
            assert!(serde_json::from_str::<MealAddition>(&q).is_ok(), "{word}");
            assert!(
                serde_json::from_str::<MealType>(&q).is_err(),
                "{word} is an addition to a meal, not a meal"
            );
        }
        assert_eq!(
            secondary.len(),
            MealAddition::ALL.len(),
            "ALL is the whole secondary tier"
        );
    }

    /// The vocabulary is closed at the wire: a word outside a field's tier — a
    /// made-up word, the right word in the wrong case, or the *other* tier's word —
    /// never reaches a handler, on create or on change. "dessert" as a meal type is
    /// the ruling made fixture: an addition *to* a meal is not a meal.
    ///
    /// "drink" is the same check for a word that *left* the vocabulary (#185): a
    /// client built against the old list is refused rather than quietly storing a
    /// word nothing downstream can fill.
    #[test]
    fn a_word_outside_the_tier_is_rejected() {
        for bad in [
            r#"{"meal_type":"brunch"}"#,
            r#"{"meal_type":"Dinner"}"#,
            r#"{"meal_type":"dessert"}"#,
            r#"{"meal_type":"side"}"#,
            r#"{"meal_type":"drink"}"#,
            r#"{"additions":["dinner"]}"#,
            r#"{"additions":["Dessert"]}"#,
            r#"{"additions":["drink"]}"#,
            r#"{"additions":["dessert","nonsense"]}"#,
        ] {
            assert!(serde_json::from_str::<CreateBody>(bad).is_err(), "{bad}");
        }
        for bad in [r#"{"meal_type":"brunch"}"#, r#"{"meal_type":"dessert"}"#] {
            assert!(serde_json::from_str::<MealTypeBody>(bad).is_err(), "{bad}");
        }
        for bad in [
            r#"{"additions":["dinner"]}"#,
            r#"{"additions":["drink"]}"#,
            r#"{"additions":["dessert","nonsense"]}"#,
        ] {
            assert!(serde_json::from_str::<AdditionsBody>(bad).is_err(), "{bad}");
        }
    }

    /// A plan carries what comes with the meal: stored once each, in vocabulary
    /// order, however noisily the client said it — the list means a set.
    #[tokio::test]
    async fn additions_are_stored_deduped_in_vocabulary_order() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[
                MealAddition::Side,
                MealAddition::Dessert,
                MealAddition::Side,
            ],
            None,
        )
        .await
        .unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(
            view.additions,
            vec![MealAddition::Dessert, MealAddition::Side],
            "once each, dessert before side"
        );
    }

    /// The host reshapes what comes with the meal while the lobby is open; the
    /// lobby follows, and clearing is just the empty set.
    #[tokio::test]
    async fn additions_can_change_while_the_lobby_is_open() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        update_additions(&conn, "c", &[MealAddition::Side])
            .await
            .unwrap();
        assert_eq!(
            load_lobby(&conn, "c").await.unwrap().unwrap().additions,
            vec![MealAddition::Side]
        );
        update_additions(&conn, "c", &[]).await.unwrap();
        assert!(load_lobby(&conn, "c")
            .await
            .unwrap()
            .unwrap()
            .additions
            .is_empty());
    }

    /// A stored additions list outside the secondary tier is corruption — fail
    /// loud, exactly like a corrupt meal type.
    #[tokio::test]
    async fn corrupt_additions_fail_loud() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by, additions)
             VALUES ('bad', 'alice', '[\"dinner\"]')",
            (),
        )
        .await
        .unwrap();
        assert!(load_lobby(&conn, "bad").await.is_err());
    }

    /// Every writer validates, so a stored word outside the vocabulary is corruption —
    /// and the lobby fails loud rather than shrugging it into some default meal.
    #[tokio::test]
    async fn a_corrupt_meal_type_fails_loud() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by, meal_type)
             VALUES ('bad', 'alice', 'brunch')",
            (),
        )
        .await
        .unwrap();
        assert!(load_lobby(&conn, "bad").await.is_err());
    }

    /// The cap bounds (#80): `None` is "any", the presented buckets all pass, and
    /// nonsense — zero, negative, longer than a day — is refused.
    #[test]
    fn a_cap_is_validated_within_sane_bounds() {
        for ok in [
            None,
            Some(60),
            Some(1800),
            Some(3600),
            Some(7200),
            Some(86_400),
        ] {
            assert!(validate_cap(ok).is_ok(), "{ok:?} must be accepted");
        }
        for bad in [Some(0), Some(-1), Some(59), Some(86_401), Some(i64::MIN)] {
            assert!(validate_cap(bad).is_err(), "{bad:?} must be refused");
        }
    }

    /// A plan written with a time cap reads it back in the lobby; one written with
    /// `None` is uncapped.
    ///
    /// This is the *writer*, which sits below #163's default: `None` here still
    /// means "Any" and always will, because that is the value the lobby writes when
    /// a host lifts the cap, and the value every plan made before #163 carries. What
    /// changed is what the create *handler* passes when a caller names nothing — see
    /// [`default_cap`] and `main`'s `a_plan_is_born_capped_at_thirty_minutes`.
    #[tokio::test]
    async fn a_plan_carries_its_time_cap_and_none_is_any() {
        let conn = conn().await;
        create_session(
            &conn,
            "capped",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            Some(1800),
        )
        .await
        .unwrap();
        create_session(
            &conn,
            "open",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();

        let capped = load_lobby(&conn, "capped").await.unwrap().unwrap();
        assert_eq!(capped.max_total_seconds, Some(1800));
        let open = load_lobby(&conn, "open").await.unwrap().unwrap();
        assert_eq!(open.max_total_seconds, None);
    }

    /// The host can move the cap while the lobby is open, and lift it back to
    /// "any"; the lobby reads whatever the plan currently says.
    #[tokio::test]
    async fn the_cap_can_be_set_and_lifted() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        set_time_cap(&conn, "c", Some(3600)).await.unwrap();
        assert_eq!(
            load_lobby(&conn, "c")
                .await
                .unwrap()
                .unwrap()
                .max_total_seconds,
            Some(3600)
        );
        set_time_cap(&conn, "c", None).await.unwrap();
        assert_eq!(
            load_lobby(&conn, "c")
                .await
                .unwrap()
                .unwrap()
                .max_total_seconds,
            None
        );
    }

    /// Start freezes the lobby's settings *in the write*, not merely in the handler's
    /// earlier read. Those are two round trips, so a start landing between them would
    /// otherwise move the corpus bound — or what the plan is even for — out from under
    /// a plan already being swiped. Each write refuses instead, and says it wrote
    /// nothing.
    #[tokio::test]
    async fn a_started_plan_refuses_every_lobby_write() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        // Everything is still movable while the lobby is open.
        assert!(set_time_cap(&conn, "c", Some(1800)).await.unwrap());
        assert!(update_meal_type(&conn, "c", MealType::Lunch).await.unwrap());
        assert!(update_additions(&conn, "c", &[MealAddition::Dessert])
            .await
            .unwrap());

        begin_session(&conn, "c").await.unwrap();

        // …and nothing is, once the swiping is under way.
        assert!(!set_time_cap(&conn, "c", Some(3600)).await.unwrap());
        assert!(!update_meal_type(&conn, "c", MealType::Breakfast)
            .await
            .unwrap());
        assert!(!update_additions(&conn, "c", &[MealAddition::Side])
            .await
            .unwrap());

        // The frozen values are the ones the deck was dealt against.
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.max_total_seconds, Some(1800));
        assert_eq!(view.meal_type, MealType::Lunch);
        assert_eq!(view.additions, vec![MealAddition::Dessert]);
    }

    /// The walk's read of the bounds distinguishes "no such session" from "no bound":
    /// an unknown channel must surface as an error, never silently walk unbounded.
    #[tokio::test]
    async fn plan_bounds_distinguish_unknown_from_unbounded() {
        let conn = conn().await;
        assert_eq!(plan_bounds(&conn, "nope").await.unwrap(), None);
        create_session(
            &conn,
            "open",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            plan_bounds(&conn, "open").await.unwrap(),
            Some(PlanBounds::default())
        );
        create_session(
            &conn,
            "capped",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            Some(7200),
        )
        .await
        .unwrap();
        assert_eq!(
            plan_bounds(&conn, "capped").await.unwrap(),
            Some(PlanBounds {
                max_total_seconds: Some(7200),
                kitchen_id: None,
                meal_type: MealType::Dinner,
            })
        );
    }

    /// The walk's read carries which meal the plan is for (#114/#184), so the deck a
    /// host is dealt is bounded by the answer they gave in the lobby — the whole point
    /// of asking. Every word of the vocabulary survives the round trip: the column is
    /// text, so a bad write and a bad read look identical from the walk's side.
    #[tokio::test]
    async fn plan_bounds_carry_the_plans_meal() {
        let conn = conn().await;
        for meal in [
            MealType::Breakfast,
            MealType::Lunch,
            MealType::Dinner,
            MealType::Snack,
        ] {
            let channel = format!("plan-{}", meal.as_str());
            create_session(&conn, &channel, "alice", None, None, meal, &[], None)
                .await
                .unwrap();
            assert_eq!(
                plan_bounds(&conn, &channel).await.unwrap(),
                Some(PlanBounds {
                    max_total_seconds: None,
                    kitchen_id: None,
                    meal_type: meal,
                }),
                "a {meal:?} plan bounds its walk to {meal:?}"
            );
        }
    }

    /// A stored word outside the vocabulary is corruption, and the bounds read fails
    /// loud rather than falling back to the default — the same ruling `load_lobby`
    /// already makes. Falling back would be worse here than in the lobby: the lobby at
    /// least shows the wrong word, while a silently-defaulted bound deals a deck for a
    /// meal nobody chose and says nothing.
    #[tokio::test]
    async fn plan_bounds_refuse_a_corrupt_meal_type() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by, meal_type)
             VALUES ('c', 'alice', 'brunch')",
            (),
        )
        .await
        .unwrap();
        let err = plan_bounds(&conn, "c").await.unwrap_err().to_string();
        assert!(err.contains("brunch"), "the bad word is named: {err}");
    }

    // ---- the kitchen limit (#82) -------------------------------------------

    /// A kitchen, optionally with equipment recorded against it.
    async fn kitchen(conn: &Connection, items: &[&str]) -> String {
        let kid = crate::kitchens::create_kitchen(conn, "Home", "alice")
            .await
            .unwrap();
        for item in items {
            conn.execute(
                "INSERT INTO kitchen_equipment (kitchen_id, item) VALUES (?1, ?2)",
                libsql::params![kid.clone(), *item],
            )
            .await
            .unwrap();
        }
        kid
    }

    async fn plan_for(conn: &Connection, channel: &str, kitchen_id: Option<&str>) {
        create_session(
            conn,
            channel,
            "alice",
            None,
            kitchen_id,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
    }

    /// The walk's read carries the plan's kitchen unconditionally — there is no flag to
    /// consult, because a meal planned in a kitchen is cooked in that kitchen (#82).
    #[tokio::test]
    async fn plan_bounds_carry_the_plans_kitchen() {
        let conn = conn().await;
        let kid = kitchen(&conn, &["knife"]).await;
        plan_for(&conn, "in-kitchen", Some(&kid)).await;
        plan_for(&conn, "kitchenless", None).await;

        assert_eq!(
            plan_bounds(&conn, "in-kitchen").await.unwrap(),
            Some(PlanBounds {
                max_total_seconds: None,
                kitchen_id: Some(kid),
                meal_type: MealType::Dinner,
            })
        );
        assert_eq!(
            plan_bounds(&conn, "kitchenless").await.unwrap(),
            Some(PlanBounds::default())
        );
    }

    // ---- buy checklist (#131) ----------------------------------------------

    /// The person on a check, for the tests that are about people. A check whose
    /// `by` is absent is a pantry pre-tick (#156) and belongs to the tests below it.
    fn claimant(c: &BuyCheck) -> &Voter {
        c.by.as_ref().expect("a person's tick")
    }

    /// A tick and an untick round-trip, and the read is keyed by the *recipe*:
    /// two recipes' checklists in one meal do not bleed into each other.
    #[tokio::test]
    async fn a_tick_round_trips_and_is_scoped_to_its_recipe() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        tick_item(&conn, "c", "themealdb", "52772", 2, "alice", TAP)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 0, "alice", TAP)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "99999", 0, "alice", TAP)
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(
            checks.iter().map(|c| c.index).collect::<Vec<_>>(),
            vec![0, 2],
            "in ingredient order, and only this recipe's lines"
        );

        untick_item(&conn, "c", "themealdb", "52772", 0, "alice")
            .await
            .unwrap();
        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(checks.iter().map(|c| c.index).collect::<Vec<_>>(), vec![2]);

        // The other recipe's list was never touched.
        assert_eq!(
            load_buy_checks(&conn, "c", "themealdb", "99999")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    /// The same person tapping twice is one row, and a checklist is scoped to its
    /// meal — the same recipe in another plan is another list.
    #[tokio::test]
    async fn ticking_is_idempotent_and_scoped_to_its_session() {
        let conn = conn().await;
        for channel in ["c1", "c2"] {
            started_plan(&conn, channel, &["alice"]).await;
        }
        tick_item(&conn, "c1", "themealdb", "52772", 1, "alice", TAP)
            .await
            .unwrap();
        tick_item(&conn, "c1", "themealdb", "52772", 1, "alice", TAP)
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c1", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(checks.len(), 1, "one line, one tick");
        assert_eq!(claimant(&checks[0]).telegram_user_id, "alice");
        assert!(
            load_buy_checks(&conn, "c2", "themealdb", "52772")
                .await
                .unwrap()
                .is_empty(),
            "another plan's checklist is its own"
        );
    }

    /// One item, one claimant: a second person tapping the same line takes it over
    /// rather than appearing beside the first. Last writer wins.
    #[tokio::test]
    async fn a_second_ticker_takes_the_item_over() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        tick_item(&conn, "c", "themealdb", "52772", 3, "alice", TAP)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 3, "bob", TAP)
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(checks.len(), 1, "never two people on one ingredient");
        assert_eq!(claimant(&checks[0]).telegram_user_id, "bob");
    }

    /// Unticking a line nobody had is the state the caller asked for, not an error.
    #[tokio::test]
    async fn unticking_an_untouched_line_is_fine() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        untick_item(&conn, "c", "themealdb", "52772", 7, "alice")
            .await
            .unwrap();
        assert!(load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap()
            .is_empty());
    }

    /// A tick carries the whole person, handle and all — the checklist says "@dave
    /// got the flour", so the read has to reach the far column of the users join.
    /// A ticker with no handle still reads back, by id.
    #[tokio::test]
    async fn a_tick_carries_who_ticked_it() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)",
            libsql::params!["4242", "dave"],
        )
        .await
        .unwrap();
        started_plan(&conn, "c", &["4242", "5150"]).await;
        tick_item(&conn, "c", "themealdb", "52772", 0, "4242", TAP)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 1, "5150", TAP)
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(claimant(&checks[0]).telegram_user_id, "4242");
        assert_eq!(claimant(&checks[0]).username.as_deref(), Some("dave"));
        assert_eq!(claimant(&checks[1]).telegram_user_id, "5150");
        assert_eq!(
            claimant(&checks[1]).username,
            None,
            "a Telegram account need not have a handle"
        );
        assert!(
            checks.iter().all(|c| c.pantry.is_none()),
            "a person's tick is not the pantry's"
        );
    }

    // ---- who may write a shopping claim (#175) ------------------------------

    /// A claim is written under the same rule as a vote: a seat, at a plan under way.
    /// A shopping list is reached *through* a decision, so a plan still in its lobby
    /// has nothing to shop for; and the roster is who is having this meal, so holding
    /// the channel id is not a licence to fill their basket.
    #[tokio::test]
    async fn a_shopping_claim_needs_a_seat_at_a_started_plan() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();

        tick_item(&conn, "c", "themealdb", "52772", 0, "alice", TAP)
            .await
            .unwrap();
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "52772")
                .await
                .unwrap()
                .is_empty(),
            "nothing is decided yet, so there is no list to tick"
        );

        begin_session(&conn, "c").await.unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 0, "mallory", TAP)
            .await
            .unwrap();
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "52772")
                .await
                .unwrap()
                .is_empty(),
            "and a stranger with the channel id is not one of the shoppers"
        );

        tick_item(&conn, "c", "themealdb", "52772", 0, "alice", TAP)
            .await
            .unwrap();
        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(claimant(&checks[0]).telegram_user_id, "alice");
    }

    /// Anyone deciding this meal may put anything back; nobody else may. Clearing is a
    /// write on a shared list, so it carries the same predicate the tick does — a
    /// guarded claim with an unguarded release is not guarded.
    #[tokio::test]
    async fn only_a_decider_can_put_a_line_back() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        tick_item(&conn, "c", "themealdb", "52772", 0, "alice", TAP)
            .await
            .unwrap();

        untick_item(&conn, "c", "themealdb", "52772", 0, "mallory")
            .await
            .unwrap();
        assert_eq!(
            load_buy_checks(&conn, "c", "themealdb", "52772")
                .await
                .unwrap()
                .len(),
            1,
            "a stranger holding the channel id does not empty somebody's basket"
        );

        untick_item(&conn, "c", "themealdb", "52772", 0, "bob")
            .await
            .unwrap();
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "52772")
                .await
                .unwrap()
                .is_empty(),
            "but the person beside you can — a shopping list is a shared object"
        );
    }

    /// The guard is in the write, not in the caller's read.
    ///
    /// The event framework checks the roster at its choke point and *then* runs the
    /// handler that writes — two round trips — so what the write is judged against is
    /// the roster as it stands when it lands, not the one the guard saw. Deleting the
    /// seat directly is that state arriving between the two.
    ///
    /// Today no API path can produce it: `remove_voter` refuses a departure after the
    /// start, and a claim cannot be written before it, so the two guards make each
    /// other's race unreachable. That is the reason to pin it rather than to skip it —
    /// a guard that only holds while a *different* guard holds is the kind that
    /// quietly stops holding when the other one moves.
    #[tokio::test]
    async fn a_tick_is_judged_against_the_roster_at_write_time() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        conn.execute(
            "DELETE FROM pick_voters WHERE channel_id = 'c' AND user_id = 'bob'",
            (),
        )
        .await
        .unwrap();

        tick_item(&conn, "c", "themealdb", "52772", 0, "bob", TAP)
            .await
            .unwrap();
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "52772")
                .await
                .unwrap()
                .is_empty(),
            "the seat was gone by the time the write arrived, so the write loses"
        );
    }

    // ---- the pantry seed (#156) --------------------------------------------

    /// A kitchen stocked with `pantry`, and a plan in it — the whole precondition for
    /// a pre-tick. The entries go in raw, exactly as `kitchens::add_pantry` writes them
    /// (normalised names taken from the corpus vocabulary).
    ///
    /// The plan is **under way**, with alice deciding it: a shopping list is only
    /// reached through a decision, so that is the state these tests are about (#175).
    async fn stocked_plan(conn: &Connection, channel: &str, pantry: &[&str]) -> String {
        let kid = crate::kitchens::create_kitchen(conn, "Home", "alice")
            .await
            .unwrap();
        for item in pantry {
            conn.execute(
                "INSERT INTO kitchen_pantry (kitchen_id, item) VALUES (?1, ?2)",
                libsql::params![kid.clone(), *item],
            )
            .await
            .unwrap();
        }
        plan_for(conn, channel, Some(&kid)).await;
        seat_voter(conn, channel, "alice").await.unwrap();
        begin_session(conn, channel).await.unwrap();
        kid
    }

    /// A recipe in the corpus as `derive` stores it: a raw name and measure with the
    /// enrich worker's reading beside it (#11). A `None` reading is an unread line,
    /// which takes no index on the shopping list — exactly what a seed must get right.
    async fn corpus_recipe(
        conn: &Connection,
        source: &str,
        id: &str,
        lines: &[(&str, Option<&str>)],
    ) {
        let ingredients: Vec<recipe_core::Ingredient> = lines
            .iter()
            .map(|(name, reading)| recipe_core::Ingredient {
                name: (*name).to_owned(),
                measure: None,
                structured: reading.map(|item| recipe_core::StructuredMeasure {
                    item: item.to_owned(),
                    amount: None,
                    preparation: None,
                    note: None,
                }),
            })
            .collect();
        conn.execute(
            "INSERT INTO recipes (source, id, title, image, category, area, tags, ingredients, instructions, source_url, video_url)
             VALUES (?1, ?2, 'Chicken Handi', NULL, NULL, NULL, '[]', ?3, '', NULL, NULL)",
            libsql::params![source, id, serde_json::to_string(&ingredients).unwrap()],
        )
        .await
        .unwrap();
    }

    /// The whole feature in one pass: the kitchen's staples arrive already ticked, on
    /// the right lines, with nobody's name on them and the jar that answered named.
    #[tokio::test]
    async fn a_stocked_kitchen_pre_ticks_the_lines_it_covers() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt", "olive oil", "onion"]).await;
        corpus_recipe(
            &conn,
            "themealdb",
            "52795",
            &[
                ("Chicken", Some("chicken")),
                ("Onion", Some("Onions")),
                ("A splash of something", None),
                ("Salt", Some("salt")),
                ("Spring onions", Some("spring onions")),
            ],
        )
        .await;

        let pantry = plan_pantry(&conn, "c").await.unwrap();
        let names = shopping_names_of(&conn, "themealdb", "52795")
            .await
            .unwrap()
            .expect("the recipe is in the corpus");
        // The unread line took no index, so "Salt" is line 2 of the checklist.
        assert_eq!(names, vec!["chicken", "Onions", "salt", "spring onions"]);

        let preticks = recipe_core::pantry::preticks(&names, &pantry);
        assert_eq!(
            preticks,
            vec![(1, "onion".to_string()), (2, "salt".to_string())],
            "onion covers Onions and salt covers Salt; chicken and spring onions are not in this pantry"
        );

        write_seed(&conn, "c", "themealdb", "52795", &preticks, TAP)
            .await
            .unwrap();
        let checks = load_buy_checks(&conn, "c", "themealdb", "52795")
            .await
            .unwrap();
        assert_eq!(
            checks.iter().map(|c| c.index).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(
            checks.iter().all(|c| c.by.is_none()),
            "nobody claimed these, so nobody's colour goes on them"
        );
        assert_eq!(
            checks.iter().map(|c| c.pantry.clone()).collect::<Vec<_>>(),
            vec![Some("onion".to_string()), Some("salt".to_string())],
            "and each says which jar answered for it"
        );
    }

    /// A plan with no kitchen has no pantry — 6 of the 9 plans in production today.
    /// That is an empty pantry, never an error and never a reason to refuse a list.
    #[tokio::test]
    async fn a_plan_without_a_kitchen_pre_ticks_nothing() {
        let conn = conn().await;
        plan_for(&conn, "c", None).await;
        assert!(plan_pantry(&conn, "c").await.unwrap().is_empty());
    }

    /// A kitchen with an empty pantry — every kitchen in production today, so this is
    /// the shape the feature actually meets on the live site. Inert, not broken.
    #[tokio::test]
    async fn an_unstocked_kitchen_pre_ticks_nothing() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &[]).await;
        assert!(plan_pantry(&conn, "c").await.unwrap().is_empty());
    }

    /// A recipe the corpus does not hold is not a list yet, and `None` says so — as
    /// against a recipe that *is* held but unread, whose list is genuinely empty. The
    /// seed treats the two differently: nothing is recorded for the first, so a list
    /// built after a derive still gets its seed.
    #[tokio::test]
    async fn an_absent_recipe_is_not_an_empty_list() {
        let conn = conn().await;
        assert_eq!(
            shopping_names_of(&conn, "themealdb", "99999")
                .await
                .unwrap(),
            None
        );
        corpus_recipe(&conn, "themealdb", "52795", &[("Salt", None)]).await;
        assert_eq!(
            shopping_names_of(&conn, "themealdb", "52795")
                .await
                .unwrap(),
            Some(vec![]),
            "held but unread — a list with no lines to match"
        );
    }

    /// The seed happens **once**. Unticking a pre-tick is an ordinary untick, and the
    /// pantry does not put the empty jar back on the next read — which is the whole
    /// reason `buy_seeds` is a table rather than "seed when the list looks empty".
    #[tokio::test]
    async fn unticking_a_pre_tick_sticks() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt"]).await;
        let preticks = vec![(0usize, "salt".to_string())];

        assert!(!buy_list_seeded(&conn, "c", "themealdb", "52795")
            .await
            .unwrap());
        write_seed(&conn, "c", "themealdb", "52795", &preticks, TAP)
            .await
            .unwrap();
        assert!(buy_list_seeded(&conn, "c", "themealdb", "52795")
            .await
            .unwrap());

        untick_item(&conn, "c", "themealdb", "52795", 0, "alice")
            .await
            .unwrap();
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "52795")
                .await
                .unwrap()
                .is_empty(),
            "the jar was empty; the untick is an untick"
        );
    }

    /// Ticking a pre-ticked line makes it yours, colour and all — the pantry's claim is
    /// replaced, not stacked on. The schema would refuse a row holding both.
    #[tokio::test]
    async fn taking_over_a_pre_tick_makes_it_a_persons() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt"]).await;
        write_seed(
            &conn,
            "c",
            "themealdb",
            "52795",
            &[(0usize, "salt".to_string())],
            TAP,
        )
        .await
        .unwrap();

        tick_item(&conn, "c", "themealdb", "52795", 0, "alice", TAP)
            .await
            .unwrap();
        let checks = load_buy_checks(&conn, "c", "themealdb", "52795")
            .await
            .unwrap();
        assert_eq!(checks.len(), 1, "one line, one tick");
        assert_eq!(claimant(&checks[0]).telegram_user_id, "alice");
        assert_eq!(
            checks[0].pantry, None,
            "a person's claim replaces the cupboard's, it does not sit beside it"
        );
    }

    /// The seed never overwrites a claim. If somebody ticks a line in the same breath
    /// as the seed runs, the person keeps it.
    #[tokio::test]
    async fn the_seed_does_not_overwrite_a_persons_tick() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt"]).await;
        tick_item(&conn, "c", "themealdb", "52795", 0, "alice", TAP)
            .await
            .unwrap();
        write_seed(
            &conn,
            "c",
            "themealdb",
            "52795",
            &[(0usize, "salt".to_string())],
            TAP,
        )
        .await
        .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52795")
            .await
            .unwrap();
        assert_eq!(claimant(&checks[0]).telegram_user_id, "alice");
        assert_eq!(checks[0].pantry, None);
    }

    /// The seed is per (meal, recipe): a re-decided plan gets a fresh list and a fresh
    /// seed, and one meal's seed says nothing about another meal's.
    #[tokio::test]
    async fn the_seed_marker_is_per_meal_and_recipe() {
        let conn = conn().await;
        stocked_plan(&conn, "c1", &["salt"]).await;
        stocked_plan(&conn, "c2", &["salt"]).await;
        write_seed(&conn, "c1", "themealdb", "52795", &[], TAP)
            .await
            .unwrap();

        assert!(buy_list_seeded(&conn, "c1", "themealdb", "52795")
            .await
            .unwrap());
        assert!(
            !buy_list_seeded(&conn, "c1", "themealdb", "52772")
                .await
                .unwrap(),
            "another recipe in the same meal is another list"
        );
        assert!(
            !buy_list_seeded(&conn, "c2", "themealdb", "52795")
                .await
                .unwrap(),
            "another meal's list is its own"
        );
    }

    /// Seeding a list that matches nothing still records that it happened — otherwise
    /// stock added later would silently re-tick a list somebody is already shopping.
    #[tokio::test]
    async fn a_seed_that_matches_nothing_is_still_a_seed() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["saffron"]).await;
        write_seed(&conn, "c", "themealdb", "52795", &[], TAP)
            .await
            .unwrap();
        assert!(buy_list_seeded(&conn, "c", "themealdb", "52795")
            .await
            .unwrap());
        assert!(load_buy_checks(&conn, "c", "themealdb", "52795")
            .await
            .unwrap()
            .is_empty());
    }

    /// The one write on this table the roster rule deliberately does not cover (#175).
    ///
    /// A pre-tick is **nobody's** claim: it is a function of the plan's kitchen and
    /// the recipe, which is exactly why it is safe to reach from the ungated read.
    /// Gating it on a seat would be gating it on something it does not depend on, and
    /// it would put the seed behind a `started_at` the read has never needed.
    ///
    /// It is also why `remove_voter` still sweeps nothing: this is the only row a plan
    /// in its lobby can have, and nobody's claim is nobody's to give up.
    #[tokio::test]
    async fn the_pantry_seed_is_nobodys_claim_and_carries_no_roster() {
        let conn = conn().await;
        let kid = crate::kitchens::create_kitchen(&conn, "Home", "alice")
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO kitchen_pantry (kitchen_id, item) VALUES (?1, 'salt')",
            libsql::params![kid.clone()],
        )
        .await
        .unwrap();
        // Still in its lobby, and nobody seated but the host-to-be.
        plan_for(&conn, "c", Some(&kid)).await;

        write_seed(
            &conn,
            "c",
            "themealdb",
            "52795",
            &[(0usize, "salt".to_string())],
            TAP,
        )
        .await
        .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52795")
            .await
            .unwrap();
        assert_eq!(
            checks.len(),
            1,
            "the cupboard answered, roster or no roster"
        );
        assert_eq!(
            checks[0].by, None,
            "and it is nobody's, so there is no seat to check it against"
        );
        assert_eq!(checks[0].pantry.as_deref(), Some("salt"));
    }

    /// The one thing the seed *does* now ask about: the recipe (#201).
    ///
    /// #170's reasoning is untouched — a pre-tick is still nobody's claim and still
    /// depends on nothing about the caller, which is why the ungated read may reach it.
    /// This predicate is about the plan and the recipe, which is precisely what the
    /// seed is a function of, so it belongs in the write for the same reason the roster
    /// does not. What it closes is the enforcement gap #201 names: a stranger holding
    /// the channel id could name any recipe and have a kitchen's pantry read against a
    /// dinner nobody agreed to cook.
    #[tokio::test]
    async fn the_pantry_seed_only_runs_for_the_recipe_the_plan_decided() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt"]).await;
        record_vote(&conn, "c", "themealdb", "52772", "alice", true, TAP)
            .await
            .unwrap();
        decide_if_agreed(&conn, "c", "themealdb", "52772", TAP)
            .await
            .unwrap()
            .expect("alice is the whole roster, so her yes decides");

        write_seed(
            &conn,
            "c",
            "themealdb",
            "52795",
            &[(0usize, "salt".to_string())],
            TAP,
        )
        .await
        .unwrap();
        assert!(
            !buy_list_seeded(&conn, "c", "themealdb", "52795")
                .await
                .unwrap(),
            "no marker for a list this plan will never shop"
        );
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "52795")
                .await
                .unwrap()
                .is_empty(),
            "and no pre-ticks either — the cupboard was never asked"
        );

        write_seed(
            &conn,
            "c",
            "themealdb",
            "52772",
            &[(0usize, "salt".to_string())],
            TAP,
        )
        .await
        .unwrap();
        assert!(buy_list_seeded(&conn, "c", "themealdb", "52772")
            .await
            .unwrap());
        assert_eq!(
            load_buy_checks(&conn, "c", "themealdb", "52772")
                .await
                .unwrap()
                .len(),
            1,
            "the decided meal is seeded exactly as it always was"
        );
    }

    /// A row cannot claim both a person and the pantry, and cannot claim neither. The
    /// database says so, so no code path can talk it into an ambiguous tick.
    #[tokio::test]
    async fn a_tick_is_a_persons_or_the_pantrys_never_both_nor_neither() {
        let conn = conn().await;
        plan_for(&conn, "c", None).await;
        for (user, pantry) in [(Some("alice"), Some("salt")), (None, None)] {
            let refused = conn
                .execute(
                    "INSERT INTO buy_checks (channel_id, source, id, ingredient_index, user_id, pantry_item)
                     VALUES ('c', 'themealdb', '52795', 0, ?1, ?2)",
                    libsql::params![user, pantry],
                )
                .await;
            assert!(
                refused.is_err(),
                "the schema must refuse user_id={user:?} pantry_item={pantry:?}"
            );
        }
    }

    /// The tally names who said yes, not just how many — so a client rehydrating
    /// after a reconnect can still colour a card by the people who liked it.
    #[tokio::test]
    async fn the_tally_names_the_yes_voters() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob", "carol"]).await;
        record_vote(&conn, "c", "t", "a", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "a", "bob", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "a", "carol", false, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "b", "alice", false, TAP)
            .await
            .unwrap();

        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        let a = row(&rows, "a");
        assert_eq!(a.yes, 2);
        assert_eq!(a.no, 1);
        let mut voters = a.yes_voters.clone();
        voters.sort();
        assert_eq!(voters, vec!["alice".to_owned(), "bob".to_owned()]);
        assert!(
            row(&rows, "b").yes_voters.is_empty(),
            "a no is not an attribution"
        );

        // Changing your mind moves you out of the list, because a vote is a current
        // call rather than an append (`record_vote`).
        record_vote(&conn, "c", "t", "a", "bob", false, TAP)
            .await
            .unwrap();
        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(row(&rows, "a").yes_voters, vec!["alice".to_owned()]);
    }

    // ---- leaving a plan (#96) ----------------------------------------------

    /// The plumbing behind a departure: who is left, in the order the lobby lists.
    async fn roster(conn: &Connection, channel: &str) -> Vec<String> {
        match load_lobby(conn, channel).await.unwrap() {
            Some(v) => v.voters.into_iter().map(|v| v.telegram_user_id).collect(),
            None => Vec::new(),
        }
    }

    /// Leaving the lobby: the roster shrinks by exactly the person who left, and the
    /// plan carries on under the host it already had.
    #[tokio::test]
    async fn leaving_the_lobby_shrinks_the_roster() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }

        assert_eq!(
            remove_voter(&conn, "c", "bob").await.unwrap(),
            Removed::Left {
                host: "alice".to_owned()
            },
            "the host is untouched by a guest leaving"
        );
        assert_eq!(roster(&conn, "c").await, vec!["alice", "carol"]);
    }

    /// The ruling (#96): people are added to and removed from a plan **in its lobby**.
    /// Once the swiping starts the set of deciders is fixed, so a departure arriving
    /// late is refused and writes nothing at all.
    ///
    /// The guard is inside the delete's own predicate rather than a preceding read,
    /// which is what makes this hold under the race it exists for: a start landing
    /// between a handler's membership read and its write still wins.
    #[tokio::test]
    async fn leaving_after_the_start_is_refused_and_writes_nothing() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        begin_session(&conn, "c").await.unwrap();

        assert_eq!(
            remove_voter(&conn, "c", "carol").await.unwrap(),
            Removed::Started
        );
        assert_eq!(
            roster(&conn, "c").await,
            vec!["alice", "bob", "carol"],
            "the roster is what must be unchanged, not merely the answer"
        );
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.host, "alice", "and nothing was handed on");
        assert!(view.started);
    }

    /// The refusal covers the host, and it covers the last person in the plan: a
    /// started plan cannot be ended by walking out of it.
    ///
    /// Worth its own test because these are the two cases where a leave *does*
    /// something beyond shrinking a list — hand the plan on, or delete it — so a
    /// guard that only covered the ordinary guest would let one tap destroy a plan
    /// people are already swiping in.
    #[tokio::test]
    async fn a_started_plan_cannot_be_ended_or_handed_on_by_leaving() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        begin_session(&conn, "c").await.unwrap();

        // The host, who would otherwise hand it on.
        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap(),
            Removed::Started
        );
        assert_eq!(load_lobby(&conn, "c").await.unwrap().unwrap().host, "alice");
        assert_eq!(roster(&conn, "c").await, vec!["alice", "bob"]);

        // And the only person in a started solo plan, who would otherwise close it.
        create_session(
            &conn,
            "solo",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        seat_voter(&conn, "solo", "alice").await.unwrap();
        begin_session(&conn, "solo").await.unwrap();
        assert_eq!(
            remove_voter(&conn, "solo", "alice").await.unwrap(),
            Removed::Started
        );
        assert!(session_exists(&conn, "solo").await.unwrap());
        assert_eq!(roster(&conn, "solo").await, vec!["alice"]);
    }

    /// A started plan's tally can never outlive its roster.
    ///
    /// This is the invariant the client reads consensus off (`yes == deciders`), and
    /// the thing that would break it is a seat being given up after the vote cast
    /// from it — a departed yes winning a recipe "unanimously" for a group one of its
    /// voters has left, or a departed no vetoing one forever. Both are impossible
    /// now, and impossible for a structural reason rather than a swept table: votes
    /// exist only once the swiping has begun, and by then no seat can be given up.
    #[tokio::test]
    async fn a_started_tally_can_never_outlive_its_roster() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        begin_session(&conn, "c").await.unwrap();
        record_vote(&conn, "c", "t", "r1", "alice", true, TAP)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "bob", true, TAP)
            .await
            .unwrap();
        // Carol's veto — the one a departure used to be able to release.
        record_vote(&conn, "c", "t", "r1", "carol", false, TAP)
            .await
            .unwrap();

        assert_eq!(
            remove_voter(&conn, "c", "carol").await.unwrap(),
            Removed::Started
        );

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        let deciders = roster(&conn, "c").await.len();
        assert_eq!(
            participants as usize, deciders,
            "everyone who voted is still deciding"
        );
        let r1 = row(&rows, "r1");
        assert_eq!(
            (r1.yes, r1.no),
            (2, 1),
            "the veto stays with the seat behind it"
        );
        assert!(
            (r1.yes as usize) <= deciders,
            "a tally never carries more yeses than there are deciders"
        );
    }

    /// Leaving one plan is nobody else's business. The same person is in two lobbies;
    /// walking out of one leaves the other exactly as it was.
    #[tokio::test]
    async fn leaving_one_plan_leaves_the_other_alone() {
        let conn = conn().await;
        for channel in ["c1", "c2"] {
            create_session(
                &conn,
                channel,
                "alice",
                None,
                None,
                MealType::Dinner,
                &[],
                None,
            )
            .await
            .unwrap();
            for who in ["alice", "carol"] {
                seat_voter(&conn, channel, who).await.unwrap();
            }
        }

        remove_voter(&conn, "c1", "carol").await.unwrap();

        assert_eq!(roster(&conn, "c1").await, vec!["alice"]);
        assert_eq!(roster(&conn, "c2").await, vec!["alice", "carol"]);
    }

    /// The host hands the plan on rather than taking it down: it passes to the
    /// longest-standing remaining decider — the same order the lobby lists people in.
    /// Everything else about the plan survives, because a meal is not the host's to
    /// cancel once other people are gathering for it.
    #[tokio::test]
    async fn a_departing_host_hands_the_plan_on() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            None,
            MealType::Breakfast,
            &[MealAddition::Side],
            Some(1800),
        )
        .await
        .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }

        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap(),
            Removed::Left {
                host: "bob".to_owned()
            },
            "the next person in the room, not an arbitrary one"
        );
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.host, "bob");
        assert_eq!(view.voters.len(), 2);
        assert_eq!(view.meal_type, MealType::Breakfast, "the plan is unchanged");
        assert_eq!(view.additions, vec![MealAddition::Side]);
        assert_eq!(view.max_total_seconds, Some(1800));

        // And it keeps handing on, so a plan can never be left hostless.
        assert_eq!(
            remove_voter(&conn, "c", "bob").await.unwrap(),
            Removed::Left {
                host: "carol".to_owned()
            }
        );
        assert_eq!(load_lobby(&conn, "c").await.unwrap().unwrap().host, "carol");
    }

    /// A guest leaving never moves the plan's host — the UPDATE is conditional on the
    /// leaver actually holding it.
    #[tokio::test]
    async fn a_guest_leaving_does_not_move_the_host() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        assert_eq!(
            remove_voter(&conn, "c", "bob").await.unwrap(),
            Removed::Left {
                host: "alice".to_owned()
            }
        );
        assert_eq!(load_lobby(&conn, "c").await.unwrap().unwrap().host, "alice");
    }

    /// The last person out closes the plan. An empty lobby is nobody's meal, and a
    /// stale link that could still seat someone would seat them alone into a plan
    /// whose meal, additions and cap were chosen by people who all walked out.
    ///
    /// Closing it is the whole cleanup: the session row is the thing every read and
    /// every seating path gates on, so once it is gone there is nothing left to reach.
    #[tokio::test]
    async fn the_last_person_out_closes_the_plan() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();

        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap(),
            Removed::Ended,
            "no host, because there is no plan"
        );

        assert!(!session_exists(&conn, "c").await.unwrap());
        assert!(load_lobby(&conn, "c").await.unwrap().is_none());
        assert!(roster(&conn, "c").await.is_empty());
    }

    /// A join landing in the gap between "am I the last?" and the delete must win, so
    /// the delete carries its own `NOT EXISTS` rather than trusting a count read a
    /// moment earlier: with somebody else seated, the plan survives and simply shrinks.
    #[tokio::test]
    async fn a_plan_someone_else_joined_is_not_closed() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        // The race, played out: the newcomer is seated before the delete runs.
        seat_voter(&conn, "c", "newcomer").await.unwrap();

        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap(),
            Removed::Left {
                host: "newcomer".to_owned()
            }
        );
        assert!(session_exists(&conn, "c").await.unwrap());
        assert_eq!(roster(&conn, "c").await, vec!["newcomer"]);
    }

    /// Leaving is idempotent, so a retry after a lost response — or a second tap
    /// racing the first past the handler's membership read — finishes the same
    /// departure rather than starting another one.
    ///
    /// And it must not *mis-report* it. A repeat writes no seat row, exactly as a
    /// departure arriving after the start does, so the two have to be told apart:
    /// answering "this meal plan has already started" to somebody leaving a lobby
    /// that never started is a plain falsehood, and it is the answer a client sees
    /// when two taps race.
    #[tokio::test]
    async fn leaving_twice_is_the_same_departure() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap(),
            Removed::Left {
                host: "bob".to_owned()
            }
        );
        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap(),
            Removed::Left {
                host: "bob".to_owned()
            },
            "a repeat does not hand the plan on a second time, and does not claim \
             the plan started"
        );
        assert_eq!(roster(&conn, "c").await, vec!["bob"]);

        // The same again for the departure that closed the plan: the second attempt
        // still reports the plan gone, not a plan that started.
        create_session(
            &conn,
            "solo",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        seat_voter(&conn, "solo", "alice").await.unwrap();
        assert_eq!(
            remove_voter(&conn, "solo", "alice").await.unwrap(),
            Removed::Ended
        );
        assert_eq!(
            remove_voter(&conn, "solo", "alice").await.unwrap(),
            Removed::Ended
        );
    }

    /// **The socket's wire, both ways** (#208) — the shape `$lib/pick` and
    /// `$lib/session-events` mirror.
    ///
    /// Pinned here because a rename on either side of this wire is invisible to every
    /// other gate in the project: Rust compiles, `svelte-check` is happy, the stories
    /// render, and the only symptom is a timer that never appears on anybody else's
    /// phone. The two clock frames matter most — nothing renders them, so nothing would
    /// notice them going quietly wrong.
    #[test]
    fn the_event_wire_reads_and_writes_as_the_browser_expects() {
        // In: an event, carrying the initiator's own clock and no duration.
        let msg: ClientMsg = serde_json::from_str(
            r#"{"type":"event","at":1700000000000,
                 "event":{"kind":"timer_start","source":"themealdb","id":"52795","step":7}}"#,
        )
        .unwrap();
        match msg {
            ClientMsg::Event { at, event } => {
                assert_eq!(at, 1_700_000_000_000);
                assert_eq!(
                    event,
                    crate::events::SessionEvent::TimerStart {
                        source: "themealdb".into(),
                        id: "52795".into(),
                        step: 7
                    }
                );
            }
            other => panic!("read as {other:?}"),
        }

        // In: the clock measurement's answer, echoing the ping it belongs to.
        let pong: ClientMsg =
            serde_json::from_str(r#"{"type":"time_pong","server_ms":10,"client_ms":99}"#).unwrap();
        match pong {
            ClientMsg::TimePong {
                server_ms,
                client_ms,
            } => {
                assert_eq!((server_ms, client_ms), (10, 99));
            }
            other => panic!("read as {other:?}"),
        }

        // Out: the ping and the estimate it produces.
        assert_eq!(
            serde_json::to_string(&ServerMsg::TimePing { server_ms: 10 }).unwrap(),
            r#"{"type":"time_ping","server_ms":10}"#
        );
        assert_eq!(
            serde_json::to_string(&ServerMsg::TimeSync {
                offset_ms: -250,
                rtt_ms: 40
            })
            .unwrap(),
            r#"{"type":"time_sync","offset_ms":-250,"rtt_ms":40}"#
        );

        // Out: the room's timers, whole, with the deadline the client renders from.
        assert_eq!(
            serde_json::to_string(&ServerMsg::Timers {
                source: "themealdb".into(),
                id: "52795".into(),
                timers: vec![crate::timers::RunningTimer {
                    step: 7,
                    started_at: 1_700_000_000_000,
                    deadline: 1_700_000_300_000,
                    started_by: Voter {
                        telegram_user_id: "5150".into(),
                        username: Some("mel".into())
                    }
                }]
            })
            .unwrap(),
            r#"{"type":"timers","source":"themealdb","id":"52795","timers":[{"step":7,"started_at":1700000000000,"deadline":1700000300000,"started_by":{"telegram_user_id":"5150","username":"mel"}}]}"#
        );

        // In: the cook, which names nothing — the plan cooks what it decided (#211).
        let cook: ClientMsg = serde_json::from_str(
            r#"{"type":"event","at":1700000000000,"event":{"kind":"cook_started"}}"#,
        )
        .unwrap();
        match cook {
            ClientMsg::Event { at, event } => {
                assert_eq!(at, 1_700_000_000_000);
                assert_eq!(event, crate::events::SessionEvent::CookStarted {});
            }
            other => panic!("read as {other:?}"),
        }

        // Out: that the room is cooking, and since whose tap — the frame every screen
        // moves to the stove on, the initiator's included.
        assert_eq!(
            serde_json::to_string(&ServerMsg::Cooking {
                started_at: 1_700_000_000_000,
                started_by: Voter {
                    telegram_user_id: "5150".into(),
                    username: Some("mel".into())
                }
            })
            .unwrap(),
            r#"{"type":"cooking","started_at":1700000000000,"started_by":{"telegram_user_id":"5150","username":"mel"}}"#
        );
    }

    // ---- the events migrate onto the framework (#209) ------------------------

    /// A connection whose clock has been **measured**: `rtt` there and back, with this
    /// participant's clock `offset` ahead of the shared timeline. The same fixture
    /// `events`' own tests use, restated here because these tests are about what a
    /// migrated event does with the number rather than about how it is arrived at.
    fn measured(rtt: i64, offset: i64) -> crate::events::ClockOffset {
        let mut c = crate::events::ClockOffset::new();
        let t0 = 1_000_000;
        c.ping_sent(t0);
        assert!(c.pong(t0, t0 + rtt / 2 + offset, t0 + rtt));
        c
    }

    /// Raise an event the way a client does: through the framework's one choke point,
    /// with an unmeasured connection tapping at `at` and the frame arriving at `at`.
    async fn raise(
        conn: &Connection,
        channel: &str,
        who: &str,
        at: i64,
        payload: crate::events::SessionEvent,
    ) -> Vec<ServerMsg> {
        crate::events::ingest(
            conn,
            channel,
            who,
            &crate::events::ClockOffset::new(),
            at,
            at,
            payload,
        )
        .await
        .unwrap()
    }

    fn a_vote(source: &str, id: &str, vote: bool) -> crate::events::SessionEvent {
        crate::events::SessionEvent::Vote {
            source: source.to_owned(),
            id: id.to_owned(),
            vote,
        }
    }

    fn a_tick(source: &str, id: &str, index: i64, checked: bool) -> crate::events::SessionEvent {
        crate::events::SessionEvent::BuyTick {
            source: source.to_owned(),
            id: id.to_owned(),
            index,
            checked,
        }
    }

    /// One `INTEGER` column of one row, or `None` for no row and for a NULL.
    async fn number(
        conn: &Connection,
        sql: &str,
        params: impl libsql::params::IntoParams,
    ) -> Option<i64> {
        let mut rows = conn.query(sql, params).await.unwrap();
        rows.next()
            .await
            .unwrap()
            .and_then(|r| r.get::<Option<i64>>(0).unwrap())
    }

    async fn swipe_instant(conn: &Connection, channel: &str, id: &str, who: &str) -> Option<i64> {
        number(
            conn,
            "SELECT created_at_ms FROM votes
              WHERE channel_id = ?1 AND id = ?2 AND voter_id = ?3",
            libsql::params![channel, id, who],
        )
        .await
    }

    async fn tick_instant(conn: &Connection, channel: &str, id: &str, index: i64) -> Option<i64> {
        number(
            conn,
            "SELECT created_at_ms FROM buy_checks
              WHERE channel_id = ?1 AND id = ?2 AND ingredient_index = ?3",
            libsql::params![channel, id, index],
        )
        .await
    }

    // ---- when it happened ----------------------------------------------------

    /// **A swipe is dated by the swipe.** The instant recorded is the one the initiator
    /// raised the event at, not the moment its row happened to be written.
    ///
    /// The two are different columns and both are kept: `created_at` is still the row's
    /// own `unixepoch()` write time (migration 0028 is additive), and a thousandfold
    /// gap between them is exactly what says they are not the same fact.
    #[tokio::test]
    async fn a_swipe_is_dated_by_the_swipe_and_not_by_its_row() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        let tapped = 1_699_999_123_456;
        assert!(record_vote(&conn, "c", "t", "r1", "alice", true, tapped)
            .await
            .unwrap());
        assert_eq!(swipe_instant(&conn, "c", "r1", "alice").await, Some(tapped));

        let written = number(
            &conn,
            "SELECT created_at FROM votes WHERE channel_id = ?1 AND id = ?2",
            libsql::params!["c", "r1"],
        )
        .await
        .expect("the row's own write time is still recorded");
        assert!(
            written * 1000 != tapped,
            "the two columns are two facts, at two resolutions"
        );
    }

    /// A re-swipe is a person's **current** call, so the instant moves with it — the
    /// same rule `vote` itself follows, and the reason this is not an append-only log.
    #[tokio::test]
    async fn a_re_swipe_moves_the_instant_with_the_call() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        assert!(record_vote(&conn, "c", "t", "r1", "alice", true, 1_000)
            .await
            .unwrap());
        assert!(record_vote(&conn, "c", "t", "r1", "alice", false, 9_000)
            .await
            .unwrap());
        assert_eq!(swipe_instant(&conn, "c", "r1", "alice").await, Some(9_000));
    }

    /// **A tick is dated by the tap**, and a take-over re-dates it: the row records who
    /// has the flour *now* and when their hand closed on it, which is the one claim it
    /// has ever made.
    #[tokio::test]
    async fn a_tick_is_dated_by_the_tap_and_a_take_over_re_dates_it() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;

        tick_item(&conn, "c", "t", "r1", 0, "alice", 1_000)
            .await
            .unwrap();
        assert_eq!(tick_instant(&conn, "c", "r1", 0).await, Some(1_000));

        tick_item(&conn, "c", "t", "r1", 0, "bob", 7_000)
            .await
            .unwrap();
        assert_eq!(
            tick_instant(&conn, "c", "r1", 0).await,
            Some(7_000),
            "last writer wins, and the instant is the winner's"
        );
    }

    /// **The plan decided when the deciding swipe happened**, not when the UPDATE ran.
    ///
    /// A decision is the one event nobody raises: it is a consequence of the last yes,
    /// so its instant is that yes's instant and there is nothing else it could honestly
    /// be. `decided_at` — the column the win condition's `decided_at IS NULL` is asked
    /// about — is untouched beside it.
    #[tokio::test]
    async fn the_decision_happened_when_the_deciding_swipe_did() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;

        record_vote(&conn, "c", "t", "r1", "alice", true, 1_000)
            .await
            .unwrap();
        assert!(decide_if_agreed(&conn, "c", "t", "r1", 1_000)
            .await
            .unwrap()
            .is_none());

        let deciding = 1_699_999_555_000;
        record_vote(&conn, "c", "t", "r1", "bob", true, deciding)
            .await
            .unwrap();
        let decided = decide_if_agreed(&conn, "c", "t", "r1", deciding)
            .await
            .unwrap()
            .expect("the second yes completes it");
        assert_eq!(
            number(
                &conn,
                "SELECT decided_at_ms FROM pick_sessions WHERE channel_id = ?1",
                libsql::params!["c"],
            )
            .await,
            Some(deciding),
        );
        assert!(
            decided.decided_at > 0,
            "and the second-resolution column the guard is written against still stands"
        );
    }

    /// **The seed is the server's own act**, so it is stamped by the server's clock —
    /// which *is* the shared timeline, so there is nothing to normalise and nobody
    /// whose drift it would be. Nobody taps a pre-tick, and dating the kitchen's
    /// cupboard by the phone that happened to open the list would be a fiction.
    #[tokio::test]
    async fn the_seed_is_stamped_by_the_server_and_not_by_a_phone() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt"]).await;
        corpus_recipe(
            &conn,
            "themealdb",
            "52795",
            &[("Salt", Some("salt")), ("Chicken", Some("chicken"))],
        )
        .await;

        let before = crate::events::server_now_ms();
        // A tap from a phone whose clock is a decade out. The seed runs as part of
        // handling it, and takes nothing from it.
        let frames = raise(
            &conn,
            "c",
            "alice",
            1_000_000,
            a_tick("themealdb", "52795", 1, true),
        )
        .await;
        assert_eq!(frames.len(), 1);
        let after = crate::events::server_now_ms();

        let seeded = number(
            &conn,
            "SELECT seeded_at_ms FROM buy_seeds WHERE channel_id = ?1 AND id = ?2",
            libsql::params!["c", "52795"],
        )
        .await
        .expect("the seed recorded when it ran");
        assert!(
            (before..=after).contains(&seeded),
            "{seeded} is not the server's own clock ({before}..={after})"
        );
        let pretick = tick_instant(&conn, "c", "52795", 0)
            .await
            .expect("the pantry pre-ticked the salt");
        assert!((before..=after).contains(&pretick));
        assert_eq!(
            tick_instant(&conn, "c", "52795", 1).await,
            Some(1_000_000),
            "while the person's own tick keeps the instant they tapped at"
        );
    }

    // ---- normalisation, at the choke point -----------------------------------

    /// **The point of the migration.** A swipe from a phone whose clock is wildly wrong
    /// is recorded at the same shared instant as one from a phone that is right —
    /// because the framework normalises `at` through that participant's measured drift
    /// before any handler sees it.
    #[tokio::test]
    async fn a_wrong_clock_lands_a_swipe_at_the_right_instant() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;

        let real = 1_700_000_000_000;
        let right = measured(40, 0);
        let wrong = measured(40, 42 * 60 * 1000); // 42 minutes fast

        crate::events::ingest(
            &conn,
            "c",
            "alice",
            &right,
            real,
            real,
            a_vote("t", "r1", true),
        )
        .await
        .unwrap();
        crate::events::ingest(
            &conn,
            "c",
            "bob",
            &wrong,
            real + 42 * 60 * 1000,
            real,
            a_vote("t", "r1", true),
        )
        .await
        .unwrap();

        assert_eq!(swipe_instant(&conn, "c", "r1", "alice").await, Some(real));
        assert_eq!(
            swipe_instant(&conn, "c", "r1", "bob").await,
            Some(real),
            "two phones, one moment — the drift cancels"
        );
    }

    /// **The receipt is not the answer.** The same swipe delivered promptly and
    /// delivered a minute late is one instant: latency between the tap and the frame
    /// arriving must not move what the plan records.
    #[tokio::test]
    async fn latency_between_the_swipe_and_the_receipt_does_not_move_it() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let clock = measured(40, 0);

        let tapped = 1_700_000_000_000;
        crate::events::ingest(
            &conn,
            "c",
            "alice",
            &clock,
            tapped,
            tapped + 60_000,
            a_vote("t", "r1", true),
        )
        .await
        .unwrap();
        assert_eq!(swipe_instant(&conn, "c", "r1", "alice").await, Some(tapped));
    }

    /// And a tick carries the normalised instant the same way — the framework does this
    /// once, for every kind, which is the whole reason it is a framework.
    #[tokio::test]
    async fn a_tick_carries_the_normalised_instant_too() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let clock = measured(40, 5 * 60 * 1000); // five minutes fast

        let real = 1_700_000_000_000;
        crate::events::ingest(
            &conn,
            "c",
            "alice",
            &clock,
            real + 5 * 60 * 1000,
            real,
            a_tick("t", "r1", 3, true),
        )
        .await
        .unwrap();
        assert_eq!(tick_instant(&conn, "c", "r1", 3).await, Some(real));
    }

    // ---- the guard refuses exactly what it refused before ---------------------

    /// A signed-in stranger holding the invite link writes no vote and the room is told
    /// nothing — the refusal #175 put in the write, now also asked once at the choke
    /// point (#209).
    #[tokio::test]
    async fn a_non_member_swipes_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        let frames = raise(&conn, "c", "mallory", 1_000, a_vote("t", "r1", true)).await;
        assert!(frames.is_empty(), "nothing written, so nothing announced");
        assert_eq!(swipe_instant(&conn, "c", "r1", "mallory").await, None);
    }

    /// **A watcher watches** (#180/#200). Somebody who followed the link after the
    /// swiping began may read everything and may write nothing — and the room does not
    /// hear from them at all, which is the half the choke point owns: a frame announced
    /// for a write that never happened would put a phantom yes on every open client.
    #[tokio::test]
    async fn a_watcher_swipes_nothing_and_the_room_hears_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        // The plan is under way and the deciders are swiping.
        assert_eq!(
            raise(&conn, "c", "alice", 1_000, a_vote("t", "r1", true))
                .await
                .len(),
            1
        );

        let frames = raise(&conn, "c", "watcher", 2_000, a_vote("t", "r1", true)).await;
        assert!(frames.is_empty());
        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(row(&rows, "r1").yes, 1, "the watcher is not in the tally");
    }

    /// A lobby decides nothing, so it admits no swipe: the roster is still open, and a
    /// vote cast into it would be counted against a membership that can still change.
    #[tokio::test]
    async fn a_lobby_admits_no_swipe() {
        let conn = conn().await;
        super::test_support::lobby(&conn, "c", &["alice"]).await;

        let frames = raise(&conn, "c", "alice", 1_000, a_vote("t", "r1", true)).await;
        assert!(frames.is_empty());
        assert_eq!(swipe_instant(&conn, "c", "r1", "alice").await, None);
    }

    /// A decided plan's deck is over, so a swipe arriving after it is refused rather
    /// than counted (#201) — the half of the vote's precondition the framework's guard
    /// deliberately does **not** cover, because shopping happens after the decision and
    /// one predicate for both would make the decision close the shop it opened.
    #[tokio::test]
    async fn a_decided_plan_admits_no_swipe() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert_eq!(
            raise(&conn, "c", "alice", 1_000, a_vote("t", "r1", true))
                .await
                .len(),
            2,
            "one yes from a roster of one decides it: the vote, then the decision"
        );

        let frames = raise(&conn, "c", "alice", 2_000, a_vote("t", "r2", true)).await;
        assert!(frames.is_empty());
        assert_eq!(swipe_instant(&conn, "c", "r2", "alice").await, None);
    }

    /// **A watcher's shopping tick is refused, and the room is told nothing** — the
    /// test that tells the choke point apart from the write's own predicate.
    ///
    /// A tick announces the whole list *whether or not the write changed a row*, so if
    /// the guard stopped being asked here, `tick_item`'s predicate would still keep the
    /// row out and the room would still be sent a frame for a write that never
    /// happened. The two are not redundant, and this is where that shows.
    #[tokio::test]
    async fn a_watcher_ticks_nothing_and_the_room_hears_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        let frames = raise(&conn, "c", "watcher", 1_000, a_tick("t", "r1", 0, true)).await;
        assert!(
            frames.is_empty(),
            "refused at the choke point, so there is no frame at all"
        );
        assert!(load_buy_checks(&conn, "c", "t", "r1")
            .await
            .unwrap()
            .is_empty());
    }

    /// And a watcher cannot clear somebody else's claim either: a guarded claim with an
    /// unguarded release is not guarded.
    #[tokio::test]
    async fn a_watcher_unticks_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        raise(&conn, "c", "alice", 1_000, a_tick("t", "r1", 0, true)).await;

        let frames = raise(&conn, "c", "watcher", 2_000, a_tick("t", "r1", 0, false)).await;
        assert!(frames.is_empty());
        assert_eq!(
            load_buy_checks(&conn, "c", "t", "r1").await.unwrap().len(),
            1,
            "alice still has it"
        );
    }

    /// A plan still in its lobby has no list to shop (#175).
    #[tokio::test]
    async fn a_lobby_admits_no_tick() {
        let conn = conn().await;
        super::test_support::lobby(&conn, "c", &["alice"]).await;

        let frames = raise(&conn, "c", "alice", 1_000, a_tick("t", "r1", 0, true)).await;
        assert!(frames.is_empty());
        assert!(load_buy_checks(&conn, "c", "t", "r1")
            .await
            .unwrap()
            .is_empty());
    }

    /// A negative index names no line of any recipe. The guard passes — this is a
    /// decider in a started plan — so the room *is* told the list, and the list is the
    /// truth: empty.
    #[tokio::test]
    async fn a_negative_index_writes_no_row() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        let frames = raise(&conn, "c", "alice", 1_000, a_tick("t", "r1", -1, true)).await;
        match frames.as_slice() {
            [ServerMsg::Buy { checks, .. }] => assert!(checks.is_empty()),
            other => panic!("expected one buy frame, got {other:?}"),
        }
    }

    // ---- what the room is told ------------------------------------------------

    /// **The deciding swipe announces the vote and then the decision, in that order.**
    ///
    /// Both frames come out of one event, and the order is the one the socket loop held
    /// before the migration: a client reading its frames in sequence has the vote in its
    /// tally before it is told the deck is over, so the screen it lands on is never a
    /// decision against a tally that has not caught up.
    #[tokio::test]
    async fn the_deciding_swipe_announces_the_vote_and_then_the_decision() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;

        let frames = raise(&conn, "c", "alice", 1_000, a_vote("t", "r1", true)).await;
        match frames.as_slice() {
            [ServerMsg::Vote {
                voter, id, vote, ..
            }] => {
                assert_eq!((voter.as_str(), id.as_str(), *vote), ("alice", "r1", true));
            }
            other => panic!("expected one vote frame, got {other:?}"),
        }

        let frames = raise(&conn, "c", "bob", 2_000, a_vote("t", "r1", true)).await;
        match frames.as_slice() {
            [ServerMsg::Vote { voter, .. }, ServerMsg::Decided { id, .. }] => {
                assert_eq!(voter, "bob");
                assert_eq!(id, "r1");
            }
            other => panic!("expected the vote then the decision, got {other:?}"),
        }

        // And only once: a later yes on the same recipe cannot re-announce it, because
        // a decided plan admits no more votes.
        assert!(raise(&conn, "c", "alice", 3_000, a_vote("t", "r1", true))
            .await
            .is_empty());
    }

    /// A tick announces the room's **whole** list, and announces it even when the write
    /// changed nothing — the `Buy`/`Timers` rule (#131). A client that missed a frame is
    /// corrected by the next one instead of drifting, and a tap the write refused gets
    /// the basket that actually exists rather than being left showing a claim nobody
    /// holds.
    #[tokio::test]
    async fn a_tick_announces_the_whole_list_even_when_nothing_moved() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        test_support::decide(&conn, "c", "themealdb", "52772").await;

        // A tick for the recipe the plan decided against: the write's own predicate
        // refuses it (#201), and the room is still told what is in the basket.
        let frames = raise(
            &conn,
            "c",
            "alice",
            1_000,
            a_tick("themealdb", "99999", 0, true),
        )
        .await;
        match frames.as_slice() {
            [ServerMsg::Buy { source, checks, .. }] => {
                assert_eq!(source, "themealdb");
                assert!(checks.is_empty(), "refused, and the truth said out loud");
            }
            other => panic!("expected one buy frame, got {other:?}"),
        }
    }

    /// **The seed runs before the tick**, exactly as it did when a tick was an HTTP
    /// handler: a tap can be the first thing that ever touches a list, and it has to
    /// land *on top of* the pantry's answer rather than be overwritten by a seed that
    /// arrives after it.
    #[tokio::test]
    async fn a_first_tick_lands_on_top_of_the_pantry() {
        let conn = conn().await;
        stocked_plan(&conn, "c", &["salt"]).await;
        corpus_recipe(&conn, "themealdb", "52795", &[("Salt", Some("salt"))]).await;

        let frames = raise(
            &conn,
            "c",
            "alice",
            1_000,
            a_tick("themealdb", "52795", 0, true),
        )
        .await;
        match frames.as_slice() {
            [ServerMsg::Buy { checks, .. }] => {
                assert_eq!(checks.len(), 1);
                assert_eq!(
                    checks[0].by.as_ref().map(|v| v.telegram_user_id.as_str()),
                    Some("alice"),
                    "the tap wins; the pre-tick it would have got is nobody's claim"
                );
                assert_eq!(checks[0].pantry, None);
            }
            other => panic!("expected one buy frame, got {other:?}"),
        }
        assert!(
            buy_list_seeded(&conn, "c", "themealdb", "52795")
                .await
                .unwrap(),
            "and the seed still ran and recorded that it did"
        );
    }

    /// **The migrated kinds, as the browser writes them** — the shape `$lib/pick` and
    /// `$lib/session-events` mirror.
    ///
    /// Pinned for the reason the timer wire is: a rename on either side of this is
    /// invisible to every other gate in the project. Rust compiles, `svelte-check` is
    /// happy, the stories render, and the only symptom is a swipe that never counts.
    #[test]
    fn the_migrated_event_kinds_read_as_the_browser_writes_them() {
        let msg: ClientMsg = serde_json::from_str(
            r#"{"type":"event","at":1700000000000,
                 "event":{"kind":"vote","source":"themealdb","id":"52795","vote":true}}"#,
        )
        .unwrap();
        match msg {
            ClientMsg::Event { at, event } => {
                assert_eq!(at, 1_700_000_000_000);
                assert_eq!(event, a_vote("themealdb", "52795", true));
            }
            other => panic!("read as {other:?}"),
        }

        let msg: ClientMsg = serde_json::from_str(
            r#"{"type":"event","at":1700000000001,
                 "event":{"kind":"buy_tick","source":"themealdb","id":"52795",
                          "index":3,"checked":false}}"#,
        )
        .unwrap();
        match msg {
            ClientMsg::Event { at, event } => {
                assert_eq!(at, 1_700_000_000_001);
                assert_eq!(event, a_tick("themealdb", "52795", 3, false));
            }
            other => panic!("read as {other:?}"),
        }

        // The old frames are **gone**, not aliased: a client that still sends one is
        // refused rather than silently having its swipe dropped into a branch that no
        // longer exists.
        assert!(
            serde_json::from_str::<ClientMsg>(
                r#"{"type":"vote","source":"themealdb","id":"52795","vote":true}"#
            )
            .is_err(),
            "the pre-#209 vote frame is not a frame any more"
        );

        // And an initiator is still never on the wire: `deny_unknown_fields` refuses a
        // payload claiming to be somebody else rather than quietly ignoring the claim.
        assert!(serde_json::from_str::<crate::events::SessionEvent>(
            r#"{"kind":"vote","source":"t","id":"1","vote":true,"voter":"mallory"}"#
        )
        .is_err());
    }

    // ---- the cook (#211) -----------------------------------------------------

    /// A started plan of `who` that has decided on Chicken Handi — the only state a cook
    /// can be started in, built the way the deciding vote builds it rather than by
    /// writing the columns by hand.
    async fn decided_plan(conn: &Connection, channel: &str, who: &[&str]) {
        started_plan(conn, channel, who).await;
        for person in who {
            record_vote(conn, channel, "themealdb", "52795", person, true, TAP)
                .await
                .unwrap();
        }
        decide_if_agreed(conn, channel, "themealdb", "52795", TAP)
            .await
            .unwrap()
            .expect("everybody said yes, so the plan decided");
    }

    /// Raise a cook through the framework, as a browser does, and answer with the frames
    /// the room is told.
    async fn cook_event(conn: &Connection, channel: &str, who: &str, at: i64) -> Vec<ServerMsg> {
        crate::events::ingest(
            conn,
            channel,
            who,
            &crate::events::ClockOffset::new(),
            at,
            at,
            crate::events::SessionEvent::CookStarted {},
        )
        .await
        .unwrap()
    }

    /// **The cook is recorded against the plan, and the room is told** — the two halves
    /// #211 asks for, in one path.
    ///
    /// Recorded is the half a broadcast cannot supply: a plan runs for days, and a
    /// transition that existed only as a frame in flight would be lost to everybody whose
    /// browser was shut at the moment it happened.
    #[tokio::test]
    async fn starting_the_cook_is_recorded_and_announced() {
        let conn = conn().await;
        decided_plan(&conn, "c", &["alice", "bob"]).await;

        let frames = cook_event(&conn, "c", "alice", TAP).await;
        assert_eq!(frames.len(), 1, "the room is told, once");
        match &frames[0] {
            ServerMsg::Cooking {
                started_at,
                started_by,
            } => {
                assert_eq!(*started_at, TAP, "her tap, not the receipt");
                assert_eq!(started_by.telegram_user_id, "alice");
            }
            other => panic!("announced {other:?}"),
        }

        let cook = load_cook(&conn, "c")
            .await
            .unwrap()
            .expect("and it is on the plan afterwards");
        assert_eq!(cook.started_at, TAP);
        assert_eq!(cook.started_by.telegram_user_id, "alice");
    }

    /// **The rehydrate half** (#202): the cook survives every browser closing, and comes
    /// back as the same fact the room was told live.
    ///
    /// This is what `socket_loop` sends on connect, so somebody who dropped — or who came
    /// back into the plan through their kitchen (#207) — lands at the stove rather than on
    /// a shopping list nobody is shopping from.
    #[tokio::test]
    async fn a_plan_rehydrates_into_the_cook_it_started() {
        let conn = conn().await;
        decided_plan(&conn, "c", &["alice"]).await;
        assert_eq!(
            load_cook(&conn, "c").await.unwrap(),
            None,
            "a decided plan is not yet a cooking one"
        );

        cook_event(&conn, "c", "alice", TAP).await;

        // A fresh read, as a reconnecting socket makes: nothing is held in memory.
        assert_eq!(
            load_cook(&conn, "c").await.unwrap(),
            Some(Cooking {
                started_at: TAP,
                started_by: Voter {
                    telegram_user_id: "alice".to_owned(),
                    username: None,
                },
            })
        );
    }

    /// The person is joined by name, like a ticked shopping line and a running timer — a
    /// room told the cook started should not have to look up whose.
    #[tokio::test]
    async fn the_cook_carries_the_name_of_whoever_started_it() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO users (telegram_user_id, username) VALUES ('5150', 'mel')",
            (),
        )
        .await
        .unwrap();
        decided_plan(&conn, "c", &["5150"]).await;
        cook_event(&conn, "c", "5150", TAP).await;

        let cook = load_cook(&conn, "c").await.unwrap().unwrap();
        assert_eq!(cook.started_by.username.as_deref(), Some("mel"));
    }

    /// **A second tap is a no-op, not an error** — and the room is still told the truth.
    ///
    /// Both halves are the design. The recorded instant is the tap that *started* the
    /// cook and does not move, so a plan cannot restate when dinner went on every time
    /// somebody presses the button; and the announcement does not depend on the write
    /// having moved a row, so the second tapper is carried to the stove by the same frame
    /// as everybody else instead of meeting silence.
    #[tokio::test]
    async fn a_second_tap_moves_nothing_and_is_still_answered() {
        let conn = conn().await;
        decided_plan(&conn, "c", &["alice", "bob"]).await;

        assert!(
            start_cook(&conn, "c", "alice", TAP).await.unwrap(),
            "alice's tap is the one that records it"
        );
        assert!(
            !start_cook(&conn, "c", "bob", TAP + 60_000).await.unwrap(),
            "bob's changes no row"
        );

        let frames = cook_event(&conn, "c", "bob", TAP + 120_000).await;
        assert_eq!(frames.len(), 1, "and is answered all the same");
        match &frames[0] {
            ServerMsg::Cooking {
                started_at,
                started_by,
            } => {
                assert_eq!(*started_at, TAP, "the first tap still owns the instant");
                assert_eq!(started_by.telegram_user_id, "alice", "and the hand");
            }
            other => panic!("announced {other:?}"),
        }

        let cook = load_cook(&conn, "c").await.unwrap().unwrap();
        assert_eq!(cook.started_at, TAP);
        assert_eq!(cook.started_by.telegram_user_id, "alice");
    }

    /// **A watcher starts nothing** (#180/#200): not on the roster, so the framework
    /// refuses the event, nothing is written and nothing is announced — no peer's screen
    /// so much as flickers, let alone moves to the stove.
    ///
    /// The **third phase is what the choke point is for**, and it is why this kind's guard
    /// is *not* redundant with its own write predicate the way the vote's is
    /// (`events::apply` names that equivalence for the vote). This arm announces whatever
    /// is **recorded**, not only what this call wrote — so with the guard gone, a watcher
    /// tapping into a plan that is already cooking would change no row and still make the
    /// server fan a frame out to the whole room. A refusal on this socket is silent
    /// (#179/#180); one that speaks to everybody is not a refusal.
    #[tokio::test]
    async fn a_watcher_starts_no_cook_and_the_room_hears_nothing() {
        let conn = conn().await;
        decided_plan(&conn, "c", &["alice"]).await;

        let frames = cook_event(&conn, "c", "wanda", TAP).await;
        assert!(frames.is_empty(), "silence, as every refusal here is");
        assert_eq!(load_cook(&conn, "c").await.unwrap(), None);

        // And the write refuses it on its own, which is what makes the answer race-free
        // when a seat moves between the guard and the UPDATE (#175/#179).
        assert!(!start_cook(&conn, "c", "wanda", TAP).await.unwrap());
        assert_eq!(load_cook(&conn, "c").await.unwrap(), None);

        // Now the plan really is cooking, on alice's tap — and a watcher still says
        // nothing to anybody.
        assert!(start_cook(&conn, "c", "alice", TAP).await.unwrap());
        let frames = cook_event(&conn, "c", "wanda", TAP + 60_000).await;
        assert!(
            frames.is_empty(),
            "a watcher does not get to make the room's screens speak"
        );
    }

    /// **You cook the decision**: a plan still swiping has nothing to cook, so a member of
    /// it — the host, even — starts nothing and the room is told nothing.
    ///
    /// The guard that separates this kind from every other one. A cook here would move the
    /// room to a stove with no pot named on it: no recipe, no shopping list keyed to one,
    /// and a `cook` page with nothing to render.
    #[tokio::test]
    async fn an_undecided_plan_cooks_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;

        let frames = cook_event(&conn, "c", "alice", TAP).await;
        assert!(frames.is_empty());
        assert_eq!(load_cook(&conn, "c").await.unwrap(), None);
        assert!(!start_cook(&conn, "c", "alice", TAP).await.unwrap());

        // The same person, the same plan, one decision later.
        decide(&conn, "c", "themealdb", "52795").await;
        assert!(start_cook(&conn, "c", "alice", TAP).await.unwrap());
    }

    /// A lobby cooks nothing either — the swiping has not begun, so there is no roster
    /// closed behind it and nothing has been decided to cook.
    #[tokio::test]
    async fn a_lobby_cooks_nothing() {
        let conn = conn().await;
        super::test_support::lobby(&conn, "c", &["alice"]).await;
        assert!(!start_cook(&conn, "c", "alice", TAP).await.unwrap());
        assert!(cook_event(&conn, "c", "alice", TAP).await.is_empty());
    }

    /// The guard is a *whole* predicate: a stranger is refused in a decided plan, and a
    /// member is refused in an undecided one, so neither half can be dropped without
    /// something noticing.
    #[tokio::test]
    async fn the_cooks_guard_asks_both_halves() {
        let conn = conn().await;
        decided_plan(&conn, "decided", &["alice"]).await;
        started_plan(&conn, "swiping", &["alice"]).await;

        assert!(is_seated_in_a_decided_plan(&conn, "decided", "alice")
            .await
            .unwrap());
        assert!(
            !is_seated_in_a_decided_plan(&conn, "decided", "wanda")
                .await
                .unwrap(),
            "a watcher of a decided plan is still a watcher"
        );
        assert!(
            !is_seated_in_a_decided_plan(&conn, "swiping", "alice")
                .await
                .unwrap(),
            "and a decider of an undecided plan has not decided yet"
        );
    }

    /// **The initiator's tap is the event**, through the whole path: a phone whose clock
    /// is a minute fast records the same instant as everybody else's, and receipt latency
    /// does not move it.
    #[tokio::test]
    async fn the_framework_normalises_the_cooks_tap() {
        let conn = conn().await;
        decided_plan(&conn, "c", &["alice"]).await;

        // Alice's phone is a minute fast, and she tapped 200ms before the frame landed.
        let mut clock = crate::events::ClockOffset::new();
        clock.ping_sent(1_000);
        assert!(clock.pong(1_000, 1_020 + 60_000, 1_040));

        crate::events::ingest(
            &conn,
            "c",
            "alice",
            &clock,
            TAP - 200 + 60_000,
            TAP,
            crate::events::SessionEvent::CookStarted {},
        )
        .await
        .unwrap();

        let cook = load_cook(&conn, "c").await.unwrap().unwrap();
        assert_eq!(cook.started_at, TAP - 200, "her tap, not her clock");
    }

    /// Half a cook is corruption and is refused loudly rather than served — a plan
    /// cooking with nobody's hand on it, or somebody cooking at no time, is a wrong
    /// database that must not run beautifully.
    #[test]
    fn half_a_cook_is_refused() {
        assert_eq!(cook_of(None, None, None).unwrap(), None);
        assert_eq!(
            cook_of(Some(TAP), Some("alice".into()), None).unwrap(),
            Some(Cooking {
                started_at: TAP,
                started_by: Voter {
                    telegram_user_id: "alice".to_owned(),
                    username: None,
                },
            })
        );
        assert!(
            cook_of(Some(TAP), None, None).is_err(),
            "a cook with nobody"
        );
        assert!(
            cook_of(None, Some("alice".into()), None).is_err(),
            "and somebody cooking at no time"
        );
    }

    // ---- a kitchen's meals (#207) -------------------------------------------

    /// A plan for `kitchen`, called at `at`, with `who` gathered in it. `at` is stated
    /// rather than left to the clock because `created_at` is whole seconds: three plans
    /// made in one test would share a moment, and an ordering these tests cannot tell
    /// apart is an ordering they cannot pin.
    async fn kitchen_plan(conn: &Connection, channel: &str, kitchen: &str, at: i64, who: &[&str]) {
        create_session(
            conn,
            channel,
            who.first().copied().unwrap_or("alice"),
            None,
            Some(kitchen),
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        for person in who {
            seat_voter(conn, channel, person).await.unwrap();
        }
        conn.execute(
            "UPDATE pick_sessions SET created_at = ?2 WHERE channel_id = ?1",
            libsql::params![channel, at],
        )
        .await
        .unwrap();
    }

    fn channels(meals: &[KitchenMeal]) -> Vec<&str> {
        meals.iter().map(|m| m.channel_id.as_str()).collect()
    }

    /// A kitchen's meals are its own, and the newest is first — the order somebody
    /// looking for the meal they are having needs it in.
    ///
    /// The two exclusions are the same rule from both sides: another kitchen's plan is
    /// not this kitchen's business, and a plan called outside any kitchen belongs to
    /// nobody's page. Both would surface if the list were "every plan".
    #[tokio::test]
    async fn a_kitchens_meals_are_its_own_newest_first() {
        let conn = conn().await;
        kitchen_plan(&conn, "oldest", "k1", 1_000, &["alice"]).await;
        kitchen_plan(&conn, "middle", "k1", 2_000, &["alice"]).await;
        kitchen_plan(&conn, "newest", "k1", 3_000, &["alice"]).await;
        kitchen_plan(&conn, "theirs", "k2", 2_500, &["bob"]).await;
        create_session(
            &conn,
            "loose",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();
        seat_voter(&conn, "loose", "alice").await.unwrap();

        let meals = kitchen_meals(&conn, "k1").await.unwrap();
        assert_eq!(
            channels(&meals),
            vec!["newest", "middle", "oldest"],
            "newest first, and only this kitchen's"
        );
        assert_eq!(
            channels(&kitchen_meals(&conn, "k2").await.unwrap()),
            vec!["theirs"],
            "the other kitchen sees its own meal and not k1's three"
        );
    }

    /// The three states a kitchen lists, each read straight off the columns that make
    /// it: gathering, deciding, decided.
    ///
    /// The roster count is per meal, which is the half a shared count would get wrong:
    /// these three plans hold one, three and two people, and each row has to say its
    /// own number rather than the kitchen's total.
    #[tokio::test]
    async fn the_three_states_of_a_meal_read_back_as_they_are() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', '52795', 'Chicken Handi', '[]', 'Cook.')",
            (),
        )
        .await
        .unwrap();

        // Gathering: the lobby is open, one person is in, dessert comes with it.
        create_session(
            &conn,
            "gathering",
            "alice",
            None,
            Some("k1"),
            MealType::Lunch,
            &[MealAddition::Dessert],
            None,
        )
        .await
        .unwrap();
        seat_voter(&conn, "gathering", "alice").await.unwrap();
        conn.execute(
            "UPDATE pick_sessions SET created_at = 3000 WHERE channel_id = 'gathering'",
            (),
        )
        .await
        .unwrap();

        // Deciding: three in, swiping begun, nothing landed yet.
        kitchen_plan(&conn, "deciding", "k1", 2_000, &["alice", "bob", "carol"]).await;
        begin_session(&conn, "deciding").await.unwrap();

        // Decided: two in, and both said yes to the same recipe.
        kitchen_plan(&conn, "decided", "k1", 1_000, &["alice", "bob"]).await;
        begin_session(&conn, "decided").await.unwrap();
        for who in ["alice", "bob"] {
            record_vote(&conn, "decided", "themealdb", "52795", who, true, TAP)
                .await
                .unwrap();
        }
        decide_if_agreed(&conn, "decided", "themealdb", "52795", TAP)
            .await
            .unwrap()
            .expect("both of two agreeing decides it");

        let meals = kitchen_meals(&conn, "k1").await.unwrap();
        assert_eq!(channels(&meals), vec!["gathering", "deciding", "decided"]);

        assert_eq!(meals[0].meal_type, MealType::Lunch, "the plan's own word");
        assert_eq!(meals[0].additions, vec![MealAddition::Dessert]);
        assert!(!meals[0].started, "the lobby is still open");
        assert_eq!(meals[0].deciders, 1);
        assert_eq!(meals[0].decided, None);

        assert!(meals[1].started, "the swiping has begun");
        assert_eq!(meals[1].deciders, 3, "its own roster, not the kitchen's");
        assert_eq!(meals[1].decided, None, "deciding is not decided");

        assert!(meals[2].started);
        assert_eq!(meals[2].deciders, 2);
        assert_eq!(
            meals[2].decided,
            Some(DecidedMeal {
                source: "themealdb".to_owned(),
                id: "52795".to_owned(),
                title: "Chicken Handi".to_owned(),
            }),
            "a decided meal names the recipe it landed on"
        );
        assert!(
            meals.iter().all(|m| !m.cooking),
            "and not one of the three is on the hob"
        );
    }

    /// **The fourth word** (#211): a kitchen says which of its meals is cooking, and says
    /// it about that meal only.
    ///
    /// Two decided plans, one of them started, so a flag read off the wrong row — or off
    /// the kitchen rather than the plan — shows up as the quiet meal claiming the hob.
    #[tokio::test]
    async fn a_kitchen_says_which_of_its_meals_is_cooking() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', '52795', 'Chicken Handi', '[]', 'Cook.')",
            (),
        )
        .await
        .unwrap();

        for (channel, at) in [("cooking", 2_000), ("settled", 1_000)] {
            kitchen_plan(&conn, channel, "k1", at, &["alice"]).await;
            begin_session(&conn, channel).await.unwrap();
            record_vote(&conn, channel, "themealdb", "52795", "alice", true, TAP)
                .await
                .unwrap();
            decide_if_agreed(&conn, channel, "themealdb", "52795", TAP)
                .await
                .unwrap()
                .expect("the only decider agreed");
        }
        assert!(start_cook(&conn, "cooking", "alice", TAP).await.unwrap());

        let meals = kitchen_meals(&conn, "k1").await.unwrap();
        assert_eq!(channels(&meals), vec!["cooking", "settled"]);
        assert!(meals[0].cooking, "this one is on the hob");
        assert!(
            meals[1].decided.is_some() && !meals[1].cooking,
            "and this one is decided and not"
        );
    }

    /// A plan recorded as cooking with no decision behind it is corruption and is refused
    /// out loud, exactly as a decision the corpus cannot name is.
    ///
    /// `start_cook`'s own predicate makes the state unwritable — you cook the decision —
    /// so a row holding it is a wrong database, and listing it would offer a member a way
    /// into a stove with no pot named on it.
    #[tokio::test]
    async fn a_cook_on_a_plan_that_decided_nothing_is_refused() {
        let conn = conn().await;
        kitchen_plan(&conn, "c", "k1", 1_000, &["alice"]).await;
        begin_session(&conn, "c").await.unwrap();
        conn.execute(
            "UPDATE pick_sessions SET cook_started_at_ms = ?2, cook_started_by = 'alice'
              WHERE channel_id = ?1",
            libsql::params!["c", TAP],
        )
        .await
        .unwrap();

        let err = kitchen_meals(&conn, "k1")
            .await
            .expect_err("cooking without a decision is corruption");
        assert!(
            err.to_string().contains("never decided"),
            "and it says what is wrong: {err}"
        );
    }

    /// The decided title is the title of *that* recipe.
    ///
    /// Two recipes sit in the corpus and one of them was decided on, so a join keyed on
    /// the source alone — or on nothing but the row order — hands back the wrong meal
    /// while still looking like a title. That is the failure this pins: an entry naming
    /// a dish the plan did not choose is worse than one naming none.
    #[tokio::test]
    async fn a_decided_meal_carries_the_title_of_the_recipe_it_chose() {
        let conn = conn().await;
        for (id, title) in [("52795", "Chicken Handi"), ("52820", "Katsu Chicken curry")] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, ingredients, instructions)
                 VALUES ('themealdb', ?1, ?2, '[]', 'Cook.')",
                libsql::params![id, title],
            )
            .await
            .unwrap();
        }
        kitchen_plan(&conn, "c", "k1", 1_000, &["alice"]).await;
        begin_session(&conn, "c").await.unwrap();
        record_vote(&conn, "c", "themealdb", "52820", "alice", true, TAP)
            .await
            .unwrap();
        decide_if_agreed(&conn, "c", "themealdb", "52820", TAP)
            .await
            .unwrap()
            .expect("the only decider agreed");

        let meals = kitchen_meals(&conn, "k1").await.unwrap();
        assert_eq!(meals.len(), 1);
        let decided = meals[0].decided.as_ref().expect("this meal decided");
        assert_eq!(decided.id, "52820");
        assert_eq!(
            decided.title, "Katsu Chicken curry",
            "the title joined is the decided recipe's, not the other row's"
        );
    }

    /// A decision naming a recipe the corpus does not hold is refused out loud rather
    /// than listed as a meal that decided nothing.
    ///
    /// Nothing deletes from `recipes` — it is rebuilt by upsert — so this state is
    /// corruption, and a silent `None` here would render a decided plan as one still
    /// swiping: the kitchen would offer a deck for a meal that is already settled.
    #[tokio::test]
    async fn a_decision_the_corpus_cannot_name_is_refused() {
        let conn = conn().await;
        kitchen_plan(&conn, "c", "k1", 1_000, &["alice"]).await;
        begin_session(&conn, "c").await.unwrap();
        record_vote(&conn, "c", "themealdb", "52795", "alice", true, TAP)
            .await
            .unwrap();
        decide_if_agreed(&conn, "c", "themealdb", "52795", TAP)
            .await
            .unwrap()
            .expect("the only decider agreed");

        let err = kitchen_meals(&conn, "k1")
            .await
            .expect_err("a decision with no recipe row is corruption");
        assert!(
            err.to_string().contains("themealdb"),
            "and it says which recipe: {err}"
        );
    }

    /// An empty kitchen answers with an empty list, not a failure. Nobody has planned a
    /// meal here yet, and that is a state the page shows rather than an error it
    /// reports.
    #[tokio::test]
    async fn a_kitchen_with_no_meals_lists_none() {
        let conn = conn().await;
        assert_eq!(kitchen_meals(&conn, "k1").await.unwrap(), vec![]);
    }
}
