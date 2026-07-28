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

// ---- WS protocol -----------------------------------------------------------

/// A frame from a client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    /// This client's yes/no on a recipe. The voter is the authenticated session,
    /// never a field the client supplies — a client cannot vote as someone else.
    Vote {
        source: String,
        id: String,
        vote: bool,
    },
}

/// A frame to a client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    /// The full tally, sent on join so a (re)connecting client rehydrates before
    /// listening. `participants` is the distinct-voter count — the client needs it
    /// to evaluate the consensus win condition (everyone said yes).
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
    Lobby { deciders: i64, started: bool },
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
    /// The [`ServerMsg::Lobby`] and [`ServerMsg::Tally`] frames beside it already
    /// carry the *new* state — a smaller roster, and a tally with the leaver's
    /// votes gone. This frame is the **event** that explains them, and it exists
    /// because a departure can complete a consensus that was one holdout away:
    /// without it the room would watch a recipe win for no visible reason. A
    /// number that moves on its own is the thing to avoid, so the person who
    /// moved it is named.
    Left {
        /// Who went. The whole person, like [`BuyCheck::by`] — the room says a
        /// name out loud, and by the time this arrives the roster no longer holds
        /// them to look one up from.
        voter: Voter,
        /// Whether they were the last, so the plan itself is gone. Only a
        /// non-decider can still be listening at that point (the roster is empty
        /// by definition), and telling them beats leaving them swiping into a
        /// channel that no longer exists.
        ended: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
struct TallyRow {
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
/// The full vocabulary is two tiers, and the tier is the type. This enum is the
/// **primary** tier — the meals you sit down to: breakfast, lunch, dinner, a
/// snack. The **secondary** tier ([`MealAddition`]: dessert, side, drink) is the
/// things that come *with* a meal. Splitting them into two types is what makes
/// the invalid states unrepresentable: a plan's meal type simply cannot be
/// "dessert" (it would claim the whole session for something that accompanies
/// it), and a chosen addition cannot be "dinner" — serde refuses both at the
/// wire, no handler checks anything.
///
/// A **fixed vocabulary**, not free text: unlike ingredients this is a small
/// closed set, so a picker over it can be exhaustive and stable, and the coming
/// meal-type reading of the corpus can share the same words (the union of both
/// tiers). Serde owns the wire form — always the lowercase name, and an unknown
/// or wrongly-cased value is rejected at deserialization, so no handler ever
/// holds a word outside its tier. The browser sentence-cases for display; the
/// wire and the database stay lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MealType {
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

/// A secondary choice on a plan (#114): something that comes *with* the meal —
/// dessert, a side, drinks — never the meal itself. See [`MealType`] for the
/// two-tier split; this is the tier a plan can carry **several** of, alongside
/// exactly one meal.
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
    Drink,
}

impl MealAddition {
    /// Every addition, in canonical order — the order stored, and the order the
    /// picker shows.
    pub const ALL: [MealAddition; 3] = [
        MealAddition::Dessert,
        MealAddition::Side,
        MealAddition::Drink,
    ];
}

/// The canonical form of a chosen-additions list: each addition at most once, in
/// vocabulary order. The list means a *set* — "dessert and drinks" — so a double
/// tap or a reordered client must not mint a different plan.
fn normalize_additions(input: &[MealAddition]) -> Vec<MealAddition> {
    MealAddition::ALL
        .iter()
        .copied()
        .filter(|a| input.contains(a))
        .collect()
}

/// A plan that names no meal is for dinner — the meal a group most plausibly
/// plans together. The same word migration 0016 backfills, so an unstated choice
/// and a pre-migration row read identically. Not time-of-day inference: the
/// default is one fixed word, and the host changes it in the lobby if it is wrong.
impl Default for MealType {
    fn default() -> Self {
        MealType::Dinner
    }
}

impl MealType {
    /// The lowercase canonical form — what the wire carries and the DB stores.
    fn as_str(self) -> &'static str {
        match self {
            MealType::Breakfast => "breakfast",
            MealType::Lunch => "lunch",
            MealType::Dinner => "dinner",
            MealType::Snack => "snack",
        }
    }

    /// The inverse of [`Self::as_str`], for reading a stored row back. `None` for
    /// anything outside the vocabulary — the caller decides how loud to be.
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "breakfast" => MealType::Breakfast,
            "lunch" => MealType::Lunch,
            "dinner" => MealType::Dinner,
            "snack" => MealType::Snack,
            _ => return None,
        })
    }
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
    /// ([`MealType::default`]) and the host can change it in the lobby.
    #[serde(default)]
    meal_type: Option<MealType>,
    /// What comes with it (#114) — dessert, a side, drinks. Optional; none is a
    /// plain meal, and the host can add them in the lobby.
    #[serde(default)]
    additions: Vec<MealAddition>,
    /// The plan's total-time cap in seconds (#80); `None` = no cap ("Any"). Not
    /// opaque like `filter`: the walk enforces it server-side against
    /// `recipes.total_seconds`, so the backend must understand it.
    #[serde(default)]
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
                body.meal_type.unwrap_or_default(),
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

/// One ticked line of a meal's shopping list (#131): which ingredient, and whose
/// tick it is.
///
/// `by` is the whole person rather than an id because every surface that shows a
/// tick shows *who* — a bare id would make the browser join it back against the
/// roster, and a shopper who never joined the lobby (there is no such thing today,
/// but the roster is not this table's business) would render as a blank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuyCheck {
    /// The 0-based position in the recipe's ingredient list.
    pub index: i64,
    pub by: Voter,
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
/// **Leaving is allowed after the start, not only in the lobby.** That is the whole
/// point. The roster is the number a recipe has to win over (#93), so a person who
/// joined and went to bed does not merely clutter a list — every recipe now needs a
/// yes that is never coming, and a plan of three with one asleep can never decide
/// anything. Freezing the roster at start protects people already swiping from the
/// target moving *up* under them (a late joiner un-wins everything that had won);
/// nothing about that argument forbids it moving *down* when someone stops eating
/// with you. So the guarantee kept is the one that was actually made: the roster
/// never grows once the swiping starts.
///
/// A departure can therefore complete a consensus that was one holdout away, and
/// that is a real decision rather than an accident: the remaining people all said
/// yes, and the only reason it had not won was somebody who is no longer having
/// this meal. It is announced (see [`ServerMsg::Left`]) precisely so it does not
/// read as a recipe winning by itself.
///
/// Their votes and their shopping claims go with them — see [`remove_voter`]. Every
/// surface the departure moved is announced: the roster, the tally the votes left,
/// and each shopping list that lost a claim.
///
/// Guards in the house style: an unknown channel is a client bug (400), and someone
/// who was never in the plan is refused (403) rather than quietly answered, exactly
/// as [`set_buy_check`] refuses a stranger holding the channel id.
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

    let plan_ended = departed.host.is_none();
    // Tell the room before answering the leaver, so the people still in the plan and
    // the person walking out of it learn the same thing from the same act. The
    // whole-state frames go first and the event that explains them last: a client
    // that reads them in order never renders "someone left" against a roster that
    // still contains them.
    if !plan_ended {
        reload_and_announce(&state, channel).await?;
        announce_tally(&state, channel).await?;
        for (source, id) in &departed.released {
            reload_and_announce_buy(&state, channel, source, id).await?;
        }
    }
    announce_departure(&state, channel, who, plan_ended);

    Ok(Json(Departure {
        channel_id: channel.to_owned(),
        kitchen_id: view.kitchen_id,
        plan_ended,
        host: departed.host,
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
/// meal: dessert, a side, drinks.
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

/// Which recipe's checklist is being read — the plan's consensus recipe.
///
/// The recipe travels in the query rather than being looked up from the session
/// because the *decision* does not live on the session yet (the browser stashes
/// it), and inventing a second home for it here would put two answers to "what
/// did we pick" in the codebase. The checklist is keyed by recipe either way, so
/// this stays correct when the decision does move server-side.
#[derive(Debug, Deserialize)]
pub struct BuyQuery {
    pub source: String,
    pub id: String,
}

/// A tick or an untick on one line of the shopping list.
#[derive(Debug, Deserialize)]
pub struct BuyCheckBody {
    pub source: String,
    pub id: String,
    pub index: i64,
    /// `true` ticks the line (claiming it for the caller), `false` clears it.
    pub checked: bool,
}

/// `GET /api/session/{channel}/buy?source=…&id=…` — the meal's shopping checklist.
///
/// Readable by any signed-in caller holding the channel id, exactly like the lobby
/// ([`lobby`]) whose roster it names: the two answer the same question about the
/// same meal, so gating one and not the other would only mean a person who can see
/// that Mel is deciding cannot see that Mel got the carrots.
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

/// `POST /api/session/{channel}/buy` — tick a line off the shopping list, or clear it.
///
/// **Deciders only.** The roster is who is having this meal; a stranger holding the
/// channel id must not be able to write into their basket, so a non-member is
/// refused (403) rather than quietly ignored. Anyone on the roster may clear
/// anyone's tick — a shopping list is a shared object, and "I put that back" is a
/// normal thing to say out loud; the write records who has it *now*, which is the
/// only claim it ever made.
///
/// The result is the whole list, and the same list is announced to the room, so the
/// caller and every other open client land on one answer.
pub async fn set_buy_check(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<BuyCheckBody>,
) -> Result<Json<BuyList>, AppError> {
    let channel = channel.as_str();
    let body = &body;
    if body.index < 0 {
        return Err(AppError::BadRequest(format!(
            "ingredient index must not be negative, got {}",
            body.index
        )));
    }
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    if !view
        .voters
        .iter()
        .any(|v| v.telegram_user_id == user.telegram_user_id)
    {
        return Err(AppError::Forbidden(
            "only the people having this meal can tick things off its list".into(),
        ));
    }

    let user = &user;
    state
        .with_db(move |db| async move {
            if body.checked {
                tick_item(
                    &db,
                    channel,
                    &body.source,
                    &body.id,
                    body.index,
                    &user.telegram_user_id,
                )
                .await
            } else {
                untick_item(&db, channel, &body.source, &body.id, body.index).await
            }
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let list = reload_and_announce_buy(&state, channel, &body.source, &body.id).await?;
    Ok(Json(list))
}

/// Re-read one recipe's checklist and tell the room, so a tick lands on every open
/// client at once — the same shape [`reload_and_announce`] gives the lobby.
async fn reload_and_announce_buy(
    state: &AppState,
    channel: &str,
    source: &str,
    id: &str,
) -> Result<BuyList, AppError> {
    let checks = state
        .with_db(move |db| async move { load_buy_checks(&db, channel, source, id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let tx = room(&state.rooms, channel);
    if let Ok(txt) = serde_json::to_string(&ServerMsg::Buy {
        source: source.to_owned(),
        id: id.to_owned(),
        checks: checks.clone(),
    }) {
        // No receivers is an error and also a non-event: nobody is listening yet.
        let _ = tx.send(txt);
    }
    Ok(BuyList {
        channel_id: channel.to_owned(),
        source: source.to_owned(),
        id: id.to_owned(),
        checks,
    })
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
    }) {
        // No receivers is an error and also a non-event: nobody is listening yet.
        let _ = tx.send(txt);
    }
    Ok(view)
}

/// Re-read the tally and tell the room — the same frame a (re)connecting client
/// rehydrates from, sent mid-session.
///
/// Every other tally update is incremental, because every other one *adds* a vote
/// and a [`ServerMsg::Vote`] frame is enough to fold in. A departure is the one
/// thing that takes votes away (#96), and there is no negative vote frame to send —
/// so the honest correction is the whole tally, which every client already knows how
/// to replace rather than merge.
async fn announce_tally(state: &AppState, channel: &str) -> Result<(), AppError> {
    let (participants, votes) = state
        .with_db(move |db| async move { load_tally(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let tx = room(&state.rooms, channel);
    if let Ok(txt) = serde_json::to_string(&ServerMsg::Tally {
        participants,
        votes,
    }) {
        // No receivers is an error and also a non-event: nobody is listening yet.
        let _ = tx.send(txt);
    }
    Ok(())
}

/// Name whoever just left to the room (#96) — the event behind the roster and tally
/// frames that went before it. Infallible on purpose: the durable state is already
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
        }) {
            if sink.send(Message::Text(txt.into())).await.is_err() {
                return;
            }
        }
    }

    // Render's free tier closes a WS idle for 5 min; a ping well inside that keeps
    // an active session's socket — and the box — awake.
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
                    if let Ok(ClientMsg::Vote { source, id, vote }) =
                        serde_json::from_str::<ClientMsg>(&t)
                    {
                        // Durable write first, then the live push — Turso is the truth.
                        if record_vote(&db, &channel, &source, &id, &voter, vote)
                            .await
                            .is_ok()
                        {
                            if let Ok(txt) = serde_json::to_string(&ServerMsg::Vote {
                                voter: voter.clone(),
                                source,
                                id,
                                vote,
                            }) {
                                // Err only means no receivers right now — harmless.
                                let _ = tx.send(txt);
                            }
                        }
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

/// What a departure changed, so the room can be told the truth about all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Removed {
    /// Who holds the plan now; `None` when it ended with them.
    host: Option<String>,
    /// The recipes whose shopping lists lost a claim — `(source, id)` each. They are
    /// announced one whole list at a time, so a `buy` screen open on any of them
    /// stops showing a tick nobody is standing behind.
    released: Vec<(String, String)>,
}

/// Take someone out of a plan (#96), and everything that was theirs in it.
/// [`Removed::host`] is `Some` while the plan carries on and `None` once it is gone,
/// because they were the last person in it.
///
/// Idempotent from end to end — every statement is a delete or a conditional update,
/// so a retry after a lost response finishes the same departure rather than starting
/// a second one. Membership is judged by the caller's read, not by this function's
/// row counts, so two taps of the same button both succeed instead of the loser
/// being told it was never in the plan.
///
/// **Their votes go with them.** Keeping them was the tempting option — deleting
/// sounds like rewriting history — but the `votes` table is not history: a re-vote
/// already overwrites (see [`record_vote`]), so it records what the people deciding
/// currently think, not what anyone once thought. A vote is the exercise of a seat
/// on the roster; give up the seat and the vote goes with it. Keeping them breaks
/// both ways at once: a departed yes makes a recipe win "unanimously" on a roster
/// that no longer contains one of its voters, and a departed **no** is worse — it
/// vetoes a recipe forever, for a group its author is not in. That is precisely the
/// hostage-taking this endpoint exists to release.
///
/// **Their shopping claims go too** (#131). A tick is a claim on an item — "I have
/// the carrots" — so a tick held by someone who is not coming means nobody is
/// bringing carrots and the list says otherwise. Clearing it puts the line back for
/// somebody else to take, which is the ordinary untick the checklist already models
/// ("I put that back"). The failure it chooses is deliberate: if they had already
/// shopped, the cost is a duplicate bag of carrots; the other way round, dinner is
/// missing an ingredient.
///
/// **The host passes it on rather than taking the plan down.** Forbidding the host
/// to leave traps the one person who cannot escape their own plan — the hostage
/// problem again, pointed at them. Ending it means one tap destroys everyone else's
/// plan, roster, votes and shopping list, and a meal is not the host's to cancel
/// once other people are having it. Leaving it hostless leaves a lobby nobody can
/// start. So it passes to the longest-standing remaining decider — the same order
/// the lobby lists people in, so "the next person in the room" is what everybody
/// already sees. Chosen *inside* the UPDATE, and only while the leaver still holds
/// it, so there is no read-then-write gap for a second departure to slip through.
///
/// **The last person out closes the plan.** An empty plan is nobody's meal, and a
/// stale link that could still seat someone into it would seat them alone into a
/// room with a roster of one and a tally full of decisions nobody present made. The
/// delete carries its own `NOT EXISTS` condition rather than trusting a count read
/// a moment earlier — the same shape as `started_at IS NULL` on the lobby writes —
/// so a join landing in that gap wins and the plan survives.
async fn remove_voter(conn: &Connection, channel: &str, user: &str) -> anyhow::Result<Removed> {
    conn.execute(
        "DELETE FROM pick_voters WHERE channel_id = ?1 AND user_id = ?2",
        libsql::params![channel, user],
    )
    .await?;
    conn.execute(
        "DELETE FROM votes WHERE channel_id = ?1 AND voter_id = ?2",
        libsql::params![channel, user],
    )
    .await?;
    // Which lists lose a claim, read on the same connection immediately before the
    // delete that clears them — a `buy` screen open on one of those recipes has to be
    // told, and asking afterwards would only find the rows already gone.
    let mut rrows = conn
        .query(
            "SELECT DISTINCT source, id FROM buy_checks
             WHERE channel_id = ?1 AND user_id = ?2
             ORDER BY source, id",
            libsql::params![channel, user],
        )
        .await?;
    let mut released = Vec::new();
    while let Some(r) = rrows.next().await? {
        released.push((r.get::<String>(0)?, r.get::<String>(1)?));
    }
    conn.execute(
        "DELETE FROM buy_checks WHERE channel_id = ?1 AND user_id = ?2",
        libsql::params![channel, user],
    )
    .await?;

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
        // Whatever the plan accumulated goes with it. Votes can outlive their voter's
        // seat (the room takes a vote from anyone holding the channel id), so this
        // sweeps by channel rather than trusting the per-person deletes above to have
        // covered everything.
        for sql in [
            "DELETE FROM votes WHERE channel_id = ?1",
            "DELETE FROM buy_checks WHERE channel_id = ?1",
            "DELETE FROM pick_voters WHERE channel_id = ?1",
        ] {
            conn.execute(sql, libsql::params![channel]).await?;
        }
        return Ok(Removed {
            host: None,
            released,
        });
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
    // the plan is gone either way, which is what `None` says.
    let host = match rows.next().await? {
        Some(r) => Some(r.get::<String>(0)?),
        None => None,
    };
    Ok(Removed { host, released })
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
            "SELECT created_by, kitchen_id, started_at, meal_type, additions, max_total_seconds
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
        "INSERT INTO pick_sessions
            (channel_id, created_by, filter, kitchen_id, meal_type, additions, max_total_seconds)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        libsql::params![
            channel_id,
            created_by,
            filter,
            kitchen_id,
            meal_type.as_str(),
            serde_json::to_string(&normalize_additions(additions))?,
            max_total_seconds
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

/// Everything about a plan that bounds the walk it deals (#80, #82).
///
/// One struct and one read, rather than a query per facet: the walk resolves the whole
/// bound from the channel on every call, and each facet (#80's cap, #82's kitchen)
/// would otherwise be another round trip on the pick page's hot path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanBounds {
    /// The total-time cap in seconds (#80); `None` = "Any".
    pub max_total_seconds: Option<i64>,
    /// The kitchen this plan is for (#72), whose equipment limits what the walk deals
    /// (#82); `None` for a plan started outside a kitchen. Unconditional — there is no
    /// flag, because a meal planned in a kitchen is cooked in that kitchen.
    pub kitchen_id: Option<String>,
}

/// A session's bounds, for the walk: `Ok(None)` is an unknown session, `Ok(Some(..))`
/// its bounds. The two layers are deliberate — an unknown channel must surface as an
/// error to the caller, never read as "unbounded", which would hand a mistyped channel
/// the whole corpus (#80).
pub async fn plan_bounds(conn: &Connection, channel: &str) -> anyhow::Result<Option<PlanBounds>> {
    let mut rows = conn
        .query(
            "SELECT max_total_seconds, kitchen_id
             FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(PlanBounds {
        max_total_seconds: row.get(0)?,
        kitchen_id: row.get(1)?,
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

/// Record (or update) a voter's call on a recipe. Re-voting overwrites — a swipe is
/// a current decision, not an append.
async fn record_vote(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    voter: &str,
    vote: bool,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO votes (channel_id, source, id, voter_id, vote) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(channel_id, source, id, voter_id) DO UPDATE SET
            vote = excluded.vote,
            created_at = unixepoch()",
        libsql::params![channel, source, id, voter, vote as i64],
    )
    .await?;
    Ok(())
}

/// Claim one line of a meal's shopping list for `user` (#131).
///
/// Idempotent, and a take-over rather than a duplicate: the primary key does not
/// include the person, so a second tapper replaces the first (last writer wins) and
/// the timestamp moves with them. Tapping your own tick again rewrites the same row
/// to the same values.
async fn tick_item(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    index: i64,
    user: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO buy_checks (channel_id, source, id, ingredient_index, user_id)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(channel_id, source, id, ingredient_index) DO UPDATE SET
            user_id = excluded.user_id,
            created_at = unixepoch()",
        libsql::params![channel, source, id, index, user],
    )
    .await?;
    Ok(())
}

/// Put one line back on the shopping list — the tick is the row, so clearing it is a
/// delete. Deleting nothing is success: unticking something already unticked is the
/// state the caller asked for.
async fn untick_item(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    index: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM buy_checks
         WHERE channel_id = ?1 AND source = ?2 AND id = ?3 AND ingredient_index = ?4",
        libsql::params![channel, source, id, index],
    )
    .await?;
    Ok(())
}

/// One recipe's checklist in a meal: the ticked lines, in ingredient order, each with
/// the person who has it. The `users` join is a LEFT one for the same reason the
/// lobby's is — a handle is a display convenience and may be absent.
async fn load_buy_checks(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<Vec<BuyCheck>> {
    let mut rows = conn
        .query(
            "SELECT b.ingredient_index, b.user_id, u.username
             FROM buy_checks b
             LEFT JOIN users u ON u.telegram_user_id = b.user_id
             WHERE b.channel_id = ?1 AND b.source = ?2 AND b.id = ?3
             ORDER BY b.ingredient_index",
            libsql::params![channel, source, id],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(BuyCheck {
            index: r.get(0)?,
            by: Voter {
                telegram_user_id: r.get::<String>(1)?,
                username: r.get::<Option<String>>(2)?,
            },
        });
    }
    Ok(out)
}

/// The tally for a channel: distinct-voter count plus per-recipe yes/no, ranked by
/// yeses. The client derives both win conditions from this — plurality (rank by
/// `yes`) and consensus (`yes == participants && no == 0`).
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

#[cfg(test)]
mod tests {
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

    fn row<'a>(rows: &'a [TallyRow], id: &str) -> &'a TallyRow {
        rows.iter().find(|r| r.id == id).expect("a tally row")
    }

    /// Two voters, two recipes: the tally counts yes/no per recipe and the distinct
    /// voters, and ranks by yeses — enough for the client to read both plurality and
    /// consensus off it.
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
        create_session(
            &conn,
            "chan1",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[],
            None,
        )
        .await
        .unwrap();

        record_vote(&conn, "chan1", "themealdb", "r1", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "chan1", "themealdb", "r1", "bob", true)
            .await
            .unwrap();
        record_vote(&conn, "chan1", "themealdb", "r2", "alice", false)
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
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        record_vote(&conn, "c", "s", "1", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "c", "s", "1", "alice", false)
            .await
            .unwrap();

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 1);
        assert_eq!(rows.len(), 1, "one row, not two");
        assert_eq!((rows[0].yes, rows[0].no), (0, 1), "the changed-to no");
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
    /// fills `None` with [`MealType::default`], and migration 0016 backfills rows
    /// that predate the column with the same word — so a pre-#114 plan and a
    /// caller who named nothing read the same.
    #[tokio::test]
    async fn an_unstated_meal_type_is_dinner() {
        assert_eq!(MealType::default(), MealType::Dinner);

        // A body naming no meal deserializes, and resolves to the default —
        // exactly what the create handler does with it.
        let body: CreateBody = serde_json::from_str("{}").unwrap();
        assert_eq!(body.meal_type.unwrap_or_default(), MealType::Dinner);

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
        let secondary = ["dessert", "side", "drink"];
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
            r#"{"additions":["dessert","nonsense"]}"#,
        ] {
            assert!(serde_json::from_str::<CreateBody>(bad).is_err(), "{bad}");
        }
        for bad in [r#"{"meal_type":"brunch"}"#, r#"{"meal_type":"dessert"}"#] {
            assert!(serde_json::from_str::<MealTypeBody>(bad).is_err(), "{bad}");
        }
        for bad in [
            r#"{"additions":["dinner"]}"#,
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
                MealAddition::Drink,
                MealAddition::Dessert,
                MealAddition::Drink,
            ],
            None,
        )
        .await
        .unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(
            view.additions,
            vec![MealAddition::Dessert, MealAddition::Drink],
            "once each, dessert before drink"
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

    /// A plan created with a time cap reads it back in the lobby; one created
    /// without is uncapped — "Any" is the default, and every session from before
    /// the column existed stays that way.
    #[tokio::test]
    async fn a_plan_carries_its_time_cap_and_defaults_to_any() {
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
        assert!(!update_additions(&conn, "c", &[MealAddition::Drink])
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
            })
        );
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
            })
        );
        assert_eq!(
            plan_bounds(&conn, "kitchenless").await.unwrap(),
            Some(PlanBounds::default())
        );
    }

    // ---- buy checklist (#131) ----------------------------------------------

    /// A tick and an untick round-trip, and the read is keyed by the *recipe*:
    /// two recipes' checklists in one meal do not bleed into each other.
    #[tokio::test]
    async fn a_tick_round_trips_and_is_scoped_to_its_recipe() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();

        tick_item(&conn, "c", "themealdb", "52772", 2, "alice")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 0, "alice")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "99999", 0, "alice")
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

        untick_item(&conn, "c", "themealdb", "52772", 0)
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
        }
        tick_item(&conn, "c1", "themealdb", "52772", 1, "alice")
            .await
            .unwrap();
        tick_item(&conn, "c1", "themealdb", "52772", 1, "alice")
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c1", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(checks.len(), 1, "one line, one tick");
        assert_eq!(checks[0].by.telegram_user_id, "alice");
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
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 3, "alice")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 3, "bob")
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(checks.len(), 1, "never two people on one ingredient");
        assert_eq!(checks[0].by.telegram_user_id, "bob");
    }

    /// Unticking a line nobody had is the state the caller asked for, not an error.
    #[tokio::test]
    async fn unticking_an_untouched_line_is_fine() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        untick_item(&conn, "c", "themealdb", "52772", 7)
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
        create_session(&conn, "c", "4242", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 0, "4242")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 1, "5150")
            .await
            .unwrap();

        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(checks[0].by.telegram_user_id, "4242");
        assert_eq!(checks[0].by.username.as_deref(), Some("dave"));
        assert_eq!(checks[1].by.telegram_user_id, "5150");
        assert_eq!(
            checks[1].by.username, None,
            "a Telegram account need not have a handle"
        );
    }

    /// The tally names who said yes, not just how many — so a client rehydrating
    /// after a reconnect can still colour a card by the people who liked it.
    #[tokio::test]
    async fn the_tally_names_the_yes_voters() {
        let conn = conn().await;
        record_vote(&conn, "c", "t", "a", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "a", "bob", true)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "a", "carol", false)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "b", "alice", false)
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
        record_vote(&conn, "c", "t", "a", "bob", false)
            .await
            .unwrap();
        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(row(&rows, "a").yes_voters, vec!["alice".to_owned()]);
    }

    // ---- leaving a plan (#96) ----------------------------------------------

    /// The plumbing behind a departure: how many people are left, and who holds it.
    async fn roster(conn: &Connection, channel: &str) -> Vec<String> {
        match load_lobby(conn, channel).await.unwrap() {
            Some(v) => v.voters.into_iter().map(|v| v.telegram_user_id).collect(),
            None => Vec::new(),
        }
    }

    /// Leaving before the start is the easy half: the roster shrinks by exactly the
    /// person who left, and the plan carries on under the host it already had.
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
            remove_voter(&conn, "c", "bob").await.unwrap().host,
            Some("alice".to_owned()),
            "the host is untouched by a guest leaving"
        );
        assert_eq!(roster(&conn, "c").await, vec!["alice", "carol"]);
    }

    /// The crux (#96): the roster is the consensus denominator, so a departure can
    /// complete a decision that was one holdout away — and that is a decision the
    /// remaining people genuinely made.
    ///
    /// Three deciding, two already yes on the same recipe and the third silent. Before
    /// the departure `yes < deciders`, so nothing has won. After it the same two yeses
    /// are unanimous, because the person who had not answered is not eating.
    #[tokio::test]
    async fn leaving_after_the_start_can_complete_a_consensus() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        begin_session(&conn, "c").await.unwrap();
        record_vote(&conn, "c", "t", "r1", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "bob", true)
            .await
            .unwrap();

        let deciders = roster(&conn, "c").await.len() as i64;
        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(deciders, 3);
        assert!(
            row(&rows, "r1").yes < deciders,
            "one holdout short of a decision"
        );

        // Carol goes to bed. Leaving is allowed after the start precisely because
        // this is the moment a stuck plan hurts.
        remove_voter(&conn, "c", "carol").await.unwrap();

        let deciders = roster(&conn, "c").await.len() as i64;
        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(deciders, 2);
        assert_eq!(
            (row(&rows, "r1").yes, row(&rows, "r1").no),
            (2, 0),
            "unanimous over the people still having this meal"
        );
        assert!(load_lobby(&conn, "c").await.unwrap().unwrap().started);
    }

    /// A departed **yes** goes with its author, so a recipe never wins on a roster
    /// that no longer contains one of its voters — and the equality the client reads
    /// consensus off (`yes == deciders`) cannot be broken by a count that outlives
    /// the person behind it.
    #[tokio::test]
    async fn a_departed_yes_leaves_the_tally() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        for who in ["alice", "bob", "carol"] {
            record_vote(&conn, "c", "t", "r1", who, true).await.unwrap();
        }

        remove_voter(&conn, "c", "carol").await.unwrap();

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 2, "carol is not a participant any more");
        let r1 = row(&rows, "r1");
        assert_eq!((r1.yes, r1.no), (2, 0));
        assert_eq!(
            r1.yes_voters,
            vec!["alice".to_owned(), "bob".to_owned()],
            "the attribution goes with the vote"
        );
        assert_eq!(
            r1.yes as usize,
            roster(&conn, "c").await.len(),
            "a departure never leaves more yeses than deciders"
        );
    }

    /// A departed **no** goes too, which is the release valve the endpoint exists to
    /// be: a veto held by somebody who is not having this meal would block the recipe
    /// forever, for a group its author has left.
    #[tokio::test]
    async fn a_departed_no_stops_vetoing() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        record_vote(&conn, "c", "t", "r1", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "c", "t", "r1", "carol", false)
            .await
            .unwrap();

        remove_voter(&conn, "c", "carol").await.unwrap();

        let (_, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(
            (row(&rows, "r1").yes, row(&rows, "r1").no),
            (1, 0),
            "the veto left with the person holding it"
        );
    }

    /// Only the leaver's votes go. Everybody else's are untouched, including their
    /// votes on the very recipes the leaver had an opinion about, and votes in
    /// another plan entirely.
    #[tokio::test]
    async fn leaving_takes_only_your_own_votes() {
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
        for channel in ["c1", "c2"] {
            record_vote(&conn, channel, "t", "r1", "alice", true)
                .await
                .unwrap();
            record_vote(&conn, channel, "t", "r1", "carol", true)
                .await
                .unwrap();
        }

        remove_voter(&conn, "c1", "carol").await.unwrap();

        let (p1, _) = load_tally(&conn, "c1").await.unwrap();
        assert_eq!(p1, 1, "alice's vote survives carol leaving");
        let (p2, rows2) = load_tally(&conn, "c2").await.unwrap();
        assert_eq!(p2, 2, "the other plan is nobody's business");
        assert_eq!(row(&rows2, "r1").yes, 2);
    }

    /// Their shopping claims are released, not inherited (#131). A tick is a claim on
    /// an item; a claim held by someone who is not coming means nobody is bringing it,
    /// so the line goes back on the list for somebody else. Everyone else's ticks stay
    /// exactly where they were.
    #[tokio::test]
    async fn leaving_releases_your_shopping_claims() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        for who in ["alice", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }
        tick_item(&conn, "c", "themealdb", "52772", 0, "alice")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 1, "carol")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 2, "carol")
            .await
            .unwrap();
        // A second recipe she had a line on, and a third she did not.
        tick_item(&conn, "c", "themealdb", "11111", 0, "carol")
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "99999", 0, "alice")
            .await
            .unwrap();

        let removed = remove_voter(&conn, "c", "carol").await.unwrap();
        assert_eq!(
            removed.released,
            vec![
                ("themealdb".to_owned(), "11111".to_owned()),
                ("themealdb".to_owned(), "52772".to_owned()),
            ],
            "each list that lost a claim, once — and no list that did not, so the \
             room is not told about a checklist that never moved"
        );

        let checks = load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap();
        assert_eq!(
            checks.iter().map(|c| c.index).collect::<Vec<_>>(),
            vec![0],
            "carol's two lines are back on the list; alice still has hers"
        );
        assert_eq!(checks[0].by.telegram_user_id, "alice");
        assert!(
            load_buy_checks(&conn, "c", "themealdb", "11111")
                .await
                .unwrap()
                .is_empty(),
            "her only line on the other recipe is unclaimed too"
        );
        assert_eq!(
            load_buy_checks(&conn, "c", "themealdb", "99999")
                .await
                .unwrap()
                .len(),
            1,
            "a list she never touched is untouched"
        );
    }

    /// The host hands the plan on rather than taking it down: it passes to the
    /// longest-standing remaining decider — the same order the lobby lists people in.
    /// Everything else about the plan survives, because a meal is not the host's to
    /// cancel once other people are having it.
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
            &[MealAddition::Drink],
            Some(1800),
        )
        .await
        .unwrap();
        for who in ["alice", "bob", "carol"] {
            seat_voter(&conn, "c", who).await.unwrap();
        }

        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap().host,
            Some("bob".to_owned()),
            "the next person in the room, not an arbitrary one"
        );
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.host, "bob");
        assert_eq!(view.voters.len(), 2);
        assert_eq!(view.meal_type, MealType::Breakfast, "the plan is unchanged");
        assert_eq!(view.additions, vec![MealAddition::Drink]);
        assert_eq!(view.max_total_seconds, Some(1800));

        // And it keeps handing on, so a plan can never be left hostless.
        assert_eq!(
            remove_voter(&conn, "c", "bob").await.unwrap().host,
            Some("carol".to_owned())
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
            remove_voter(&conn, "c", "bob").await.unwrap().host,
            Some("alice".to_owned())
        );
        assert_eq!(load_lobby(&conn, "c").await.unwrap().unwrap().host, "alice");
    }

    /// The last person out closes the plan, and closes it completely: the session,
    /// the roster, the votes and the shopping list all go. A stale link then finds
    /// nothing to rejoin, which is the point — an empty plan must not linger as a
    /// ghost that seats a newcomer alone with decisions nobody present made.
    #[tokio::test]
    async fn the_last_person_out_closes_the_plan() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        record_vote(&conn, "c", "t", "r1", "alice", true)
            .await
            .unwrap();
        tick_item(&conn, "c", "themealdb", "52772", 0, "alice")
            .await
            .unwrap();

        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap().host,
            None,
            "no host, because there is no plan"
        );

        assert!(!session_exists(&conn, "c").await.unwrap());
        assert!(load_lobby(&conn, "c").await.unwrap().is_none());
        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!((participants, rows.len()), (0, 0));
        assert!(load_buy_checks(&conn, "c", "themealdb", "52772")
            .await
            .unwrap()
            .is_empty());
        assert!(roster(&conn, "c").await.is_empty());
    }

    /// A plan is swept clean of votes cast by people who were never on its roster
    /// either — the room takes a vote from anyone holding the channel id, so the
    /// last-one-out sweep goes by channel rather than trusting the per-person delete.
    #[tokio::test]
    async fn closing_a_plan_sweeps_votes_from_non_members_too() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[], None)
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        record_vote(&conn, "c", "t", "r1", "a-spectator", true)
            .await
            .unwrap();

        assert_eq!(remove_voter(&conn, "c", "alice").await.unwrap().host, None);
        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!((participants, rows.len()), (0, 0));
    }

    /// Leaving is idempotent from end to end, so a retry after a lost response — or
    /// a second tap — finishes the same departure rather than starting another one.
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
            remove_voter(&conn, "c", "alice").await.unwrap().host,
            Some("bob".to_owned())
        );
        assert_eq!(
            remove_voter(&conn, "c", "alice").await.unwrap().host,
            Some("bob".to_owned()),
            "a repeat does not hand the plan on a second time"
        );
        assert_eq!(roster(&conn, "c").await, vec!["bob"]);
    }
}
