//! **The plan's shared soundtrack** (#212) — the session-event framework's third
//! consumer ([`crate::events`]).
//!
//! The music was a per-device dice roll: every browser's layout picked its own random
//! track from the section's pool and started it whenever that device happened to arrive.
//! Two people shopping the same list heard different songs at different points, in a
//! feature presented as *the meal's* atmosphere. In a plan the **session owns the
//! soundtrack and devices play it back**.
//!
//! ## Two facts, and everything else is derived
//!
//! A room's music, per section, is **which track** and **the instant it started** on the
//! shared timeline. A device's playback position is `now − started_at` read through its
//! own measured clock offset — not a number anybody sends, and not a number anybody
//! stores, exactly as [`crate::timers`] derives a deadline rather than storing one.
//!
//! ## The initiator owns *when*; the room owns *what*
//!
//! This module holds the pools ([`pool`]) and makes the choice ([`choose`]). That is
//! not tidiness, it is the same rule the timers follow for durations, and it has two
//! independent reasons:
//!
//! - **One shuffle for the room.** The no-back-to-back-repeat rule is a property of the
//!   room's sequence, so it lives where the sequence does. Run per device it is N
//!   private shuffles again, which is the bug.
//! - **A track name from a client is a URL a client chose.** Every phone in the plan
//!   would load it. The wire therefore has nowhere to put one: [`crate::events::SessionEvent::MusicAdvance`]
//!   names a section and the state it is answering, never a track.
//!
//! ## Who may move it on, and how several devices agree
//!
//! A track ends on every device at about the same moment, so several of them raise the
//! rollover. The event carries `after` — **the start instant of the track that ended**,
//! which is a value that came from this module in the first place — and the write is a
//! compare-and-set on it (#205's `decided_at IS NULL` discipline, generalised): exactly
//! one call changes the row, and the losers are told what the winner chose by the frame
//! the framework announces either way. A device that slept through three tracks holds a
//! stale `after`, matches nothing, and is corrected rather than dragging the room back.
//!
//! The room's soundtrack only ever moves **forward**: a rollover stamped at or before
//! the instant the current track began is refused, so a badly-measured clock cannot make
//! the room's timeline go backwards.
//!
//! ## From the lobby, not from the start
//!
//! Music is the only thing here guarded by [`crate::events::Guard::SeatedInPlan`] rather
//! than `SeatedInStartedPlan`. Everything else on this socket writes to the *outcome* of
//! a meal — a vote, a claim on a shopping line, a countdown on a pot — and none of those
//! may be written before the roster closes. A soundtrack is not an outcome: the room
//! exists from the moment people are in the lobby waiting for the host to start, and
//! that is where they are hearing it. The boundary that matters is #200's — a watcher
//! **hears** the room and raises nothing — and that is exactly the boundary this guard
//! draws.

use libsql::Connection;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::session::seated_in_a_plan;

/// **Which leg of the meal a room's music belongs to** — the same four the nav names
/// (`$lib/types`'s `Section`), and the plan's own arc rather than the app's navigation.
///
/// A section is on the wire as its lowercase name and is stored as the same string, so
/// a row is readable without a lookup table and a frame is readable without one either.
///
/// `kitchens` is deliberately not here. It has a pool of tracks and no plan behind it —
/// standing in your own kitchen is not something a room does together — so it is served
/// by the device-local path and there is no shared state for it to have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Pick,
    Buy,
    Cook,
    Joy,
}

impl Section {
    /// The stored (and wire) spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Section::Pick => "pick",
            Section::Buy => "buy",
            Section::Cook => "cook",
            Section::Joy => "joy",
        }
    }

    /// Read a stored section back, or `None` for text this build does not know.
    ///
    /// `None` rather than a default: a row naming a section that is not in this binary
    /// is a row from another build, and the honest thing to do with it is to leave it
    /// alone — not to play its track in a section it was never chosen for.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pick" => Some(Section::Pick),
            "buy" => Some(Section::Buy),
            "cook" => Some(Section::Cook),
            "joy" => Some(Section::Joy),
            _ => None,
        }
    }
}

/// **The tracks a section can play** — the room's pool, and the only place a track name
/// a device will load ever comes from.
///
/// These are paths under the frontend's `static/`, served beside the app.
/// `every_track_the_room_can_name_is_a_file_that_exists` reads that directory, so
/// renaming an asset fails a test here rather than silencing a room in production.
///
/// **`joy` is empty on purpose** — its own tracks are still to come. An empty pool is
/// silence, which is what the section already does for a lone device, and the honest
/// state for a section whose music does not exist yet (there is no borrowing another
/// section's music to fill it: #192's rule about a deck that must never contain a
/// guess is the same rule).
pub fn pool(section: Section) -> &'static [&'static str] {
    match section {
        // One track, and it still rolls over rather than looping: see `choose`.
        Section::Pick => &["/pick.mp3"],
        Section::Buy => &[
            "/music/buy-1.mp3",
            "/music/buy-2.mp3",
            "/music/buy-3.mp3",
            "/music/buy-4.mp3",
        ],
        Section::Cook => &[
            "/music/cook-1.mp3",
            "/music/cook-2.mp3",
            "/music/cook-3.mp3",
        ],
        Section::Joy => &[],
    }
}

/// **The room's shuffle**: a track from `pool`, avoiding `exclude` so a section does not
/// repeat a song back to back.
///
/// Pure, and takes the roll rather than making it, so every branch below is pinned by a
/// test instead of being sampled at: `roll` is any number at all and the choice is
/// `roll % options.len()`.
///
/// **The no-repeat rule relaxes when there is nothing left to honour it with** — a pool
/// of one, or one whose only other members are excluded — and answers with what there
/// is. That is what makes a single-track section (`pick`) work without a second code
/// path: it rolls over to itself, at a fresh shared instant, instead of looping locally
/// on each device at whatever moment that device happened to start it.
///
/// An empty pool answers `None`, which is "this section has no music", not "silence was
/// chosen".
pub fn choose(pool: &[&'static str], exclude: Option<&str>, roll: usize) -> Option<&'static str> {
    if pool.is_empty() {
        return None;
    }
    let others: Vec<&'static str> = pool
        .iter()
        .copied()
        .filter(|t| Some(*t) != exclude)
        .collect();
    let options = if others.is_empty() { pool } else { &others[..] };
    Some(options[roll % options.len()])
}

/// One section's music, as the room holds it. Mirrors `$lib/music`'s `RoomTrack`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomTrack {
    /// Which leg of the meal this is the soundtrack for.
    pub section: Section,
    /// The track everybody in the plan is playing.
    pub track: String,
    /// **When it started**, in the shared timeline (unix ms) — the instant the previous
    /// track ended on the device that reported it, corrected for that participant's own
    /// clock drift by [`crate::events::normalize`].
    ///
    /// A device's playback position is `now − started_at`, read through its own recorded
    /// offset. Nothing stores a position and nothing sends one: two devices that agree
    /// about this instant agree about the position without ever exchanging it.
    pub started_at: i64,
}

/// A roll of the dice for [`choose`]. Not `pub`: the choice is the room's, so nothing
/// outside this module supplies the randomness behind it.
fn roll() -> usize {
    OsRng.next_u64() as usize
}

/// **Move a section's soundtrack on** — the room's next track, starting at `at`.
///
/// One function for both occasions the issue names, because they are one act with one
/// race in it:
///
/// - **`after: None` — the section has no music yet.** The first device to arrive
///   inserts; everybody else's identical claim conflicts and does nothing. A device that
///   *believes* the room is silent when it is not (a stale screen, a frame it missed) is
///   refused by the same conflict rather than restarting the room's track.
/// - **`after: Some(t)` — the track that started at `t` has ended.** The update matches
///   only while `t` is still the current start instant, so of the several devices whose
///   track ended together exactly one changes the row.
///
/// Answers whether **this** call moved the room. The framework announces the section's
/// state either way (`events::announce_music`), which is what tells the losers of the
/// race what the winner chose — the [`crate::session::ServerMsg::Buy`] whole-state rule,
/// on the state a missed frame would otherwise strand a device on.
///
/// The membership predicate is inside the write as well as at the framework's guard: a
/// seat can be given up in the round trip between the two (#175/#179).
pub async fn advance(
    conn: &Connection,
    channel: &str,
    section: Section,
    user: &str,
    after: Option<i64>,
    at: i64,
) -> anyhow::Result<bool> {
    let tracks = pool(section);
    if tracks.is_empty() {
        return Ok(false);
    }
    let written = match after {
        None => {
            let Some(track) = choose(tracks, None, roll()) else {
                return Ok(false);
            };
            conn.execute(
                &format!(
                    "INSERT INTO plan_music (channel_id, section, track, started_at_ms)
                     SELECT ?1, ?2, ?3, ?4 WHERE {}
                     ON CONFLICT(channel_id, section) DO NOTHING",
                    seated_in_a_plan("?5")
                ),
                libsql::params![channel, section.as_str(), track, at, user],
            )
            .await?
        }
        Some(after) => {
            // What is playing now, so the shuffle can avoid repeating it. Read outside
            // the write because SQLite has nowhere to put "the value this row held" in
            // an UPDATE's own expression — and it does not have to be atomic: if the
            // row moved between this read and the write below, the compare-and-set finds
            // nothing to change and this call is simply not the one that won.
            let Some(current) = current(conn, channel, section).await? else {
                return Ok(false);
            };
            let Some(track) = choose(tracks, Some(&current.track), roll()) else {
                return Ok(false);
            };
            conn.execute(
                &format!(
                    "UPDATE plan_music SET track = ?3, started_at_ms = ?4
                      WHERE channel_id = ?1 AND section = ?2
                        AND started_at_ms = ?6
                        AND ?4 > started_at_ms
                        AND {}",
                    seated_in_a_plan("?5")
                ),
                libsql::params![channel, section.as_str(), track, at, user, after],
            )
            .await?
        }
    };
    Ok(written > 0)
}

/// What one section of a plan is playing, if anything.
pub async fn current(
    conn: &Connection,
    channel: &str,
    section: Section,
) -> anyhow::Result<Option<RoomTrack>> {
    let mut rows = conn
        .query(
            "SELECT track, started_at_ms FROM plan_music
              WHERE channel_id = ?1 AND section = ?2 LIMIT 1",
            libsql::params![channel, section.as_str()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(RoomTrack {
        section,
        track: row.get(0)?,
        started_at: row.get(1)?,
    }))
}

/// **Every section this plan has music in** — the rehydrate read (#202).
///
/// A device that reconnects, or arrives hours late through the kitchen page, is handed
/// the room's current track and the instant it began, and joins it **mid-track** at the
/// position the shared timeline says. That is the whole of "a returning participant
/// joins the current track in flight": there is no separate resume, because a position
/// was never stored to resume from.
///
/// Every section, not merely the one the client is in — the [`crate::timers::load_all`]
/// rule: a client is never left holding music it was not told about, and it ignores a
/// section it is not in, the way `buy` ignores another recipe's checklist.
pub async fn load_all(conn: &Connection, channel: &str) -> anyhow::Result<Vec<RoomTrack>> {
    let mut rows = conn
        .query(
            "SELECT section, track, started_at_ms FROM plan_music
              WHERE channel_id = ?1 ORDER BY section",
            libsql::params![channel],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let stored: String = row.get(0)?;
        // A section this build does not know is left alone rather than guessed at.
        let Some(section) = Section::parse(&stored) else {
            continue;
        };
        out.push(RoomTrack {
            section,
            track: row.get(1)?,
            started_at: row.get(2)?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ingest, ClockOffset, SessionEvent};
    use crate::session::test_support::{lobby, started_plan};

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    /// A connection whose clock has been measured as `offset` ms ahead of the server.
    fn clock(offset: i64) -> ClockOffset {
        let mut c = ClockOffset::new();
        c.ping_sent(1_000);
        assert!(c.pong(1_000, 1_020 + offset, 1_040));
        c
    }

    const T0: i64 = 1_700_000_000_000;

    /// **The no-back-to-back rule, exhaustively** — every roll, not a sample of them.
    ///
    /// This is the rule that moved server-side: run per device it is N private shuffles,
    /// which is the bug #212 is about. Excluding the current track has to hold for every
    /// roll, or the room repeats a song on some of them and nobody can reproduce it.
    #[test]
    fn the_shuffle_never_repeats_the_track_that_just_ended() {
        let cook = pool(Section::Cook);
        assert_eq!(cook.len(), 3);
        for roll in 0..24 {
            let next = choose(cook, Some("/music/cook-2.mp3"), roll).unwrap();
            assert_ne!(next, "/music/cook-2.mp3", "roll {roll} repeated");
            assert!(cook.contains(&next));
        }
        // And with nothing excluded every track in the pool is reachable, so a shuffle
        // that quietly only ever dealt one of them would fail here.
        let reached: std::collections::BTreeSet<&str> =
            (0..24).map(|r| choose(cook, None, r).unwrap()).collect();
        assert_eq!(reached.len(), 3, "every track is dealt");
    }

    /// The rule **relaxes** rather than answering silence when there is nothing else:
    /// a single-track section rolls over to itself, at a fresh shared instant.
    #[test]
    fn a_lone_track_follows_itself() {
        let pick = pool(Section::Pick);
        assert_eq!(pick.len(), 1);
        for roll in 0..8 {
            assert_eq!(choose(pick, Some("/pick.mp3"), roll), Some("/pick.mp3"));
        }
    }

    /// An empty pool is "no music here", not a chosen silence — and it is `joy` today.
    #[test]
    fn an_empty_pool_chooses_nothing() {
        assert_eq!(choose(&[], None, 0), None);
        assert_eq!(choose(pool(Section::Joy), None, 7), None);
    }

    /// The roll indexes the *remaining* options, so the exclusion cannot be undone by a
    /// large number. A mutation that filters and then indexes the unfiltered pool
    /// survives every small roll and fails here.
    #[test]
    fn the_roll_indexes_what_is_left_after_the_exclusion() {
        let buy = pool(Section::Buy);
        for roll in [0usize, 1, 2, 3, 4, 97, 1_000_003, usize::MAX] {
            let next = choose(buy, Some("/music/buy-1.mp3"), roll).unwrap();
            assert_ne!(next, "/music/buy-1.mp3", "roll {roll}");
        }
    }

    /// **Every track the room can name is a file that is actually served.**
    ///
    /// The pools live here because the choice is the room's, and the files live in the
    /// frontend's `static/` because that is what serves them. A rename on one side is
    /// otherwise a room told to play a 404 — silence that looks exactly like the bug
    /// this feature fixes.
    #[test]
    fn every_track_the_room_can_name_is_a_file_that_exists() {
        let static_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../frontend/static")
            .canonicalize()
            .expect("the frontend's static directory");
        for section in [Section::Pick, Section::Buy, Section::Cook, Section::Joy] {
            for track in pool(section) {
                let path = static_dir.join(track.trim_start_matches('/'));
                assert!(
                    path.is_file(),
                    "{} names {track}, which is not a file under frontend/static",
                    section.as_str()
                );
            }
        }
    }

    /// A section is stored and read back as itself; unknown text is not guessed at.
    #[test]
    fn a_section_round_trips_and_an_unknown_one_is_refused() {
        for section in [Section::Pick, Section::Buy, Section::Cook, Section::Joy] {
            assert_eq!(Section::parse(section.as_str()), Some(section));
        }
        assert_eq!(Section::parse("kitchens"), None, "not a leg of the meal");
        assert_eq!(Section::parse(""), None);
    }

    /// The first device to arrive starts the section, and the room's music is that
    /// track from that instant.
    #[tokio::test]
    async fn the_first_arrival_starts_the_section() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert!(advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap());

        let now = current(&conn, "c", Section::Cook).await.unwrap().unwrap();
        assert_eq!(now.section, Section::Cook);
        assert_eq!(now.started_at, T0, "the initiator's instant");
        assert!(pool(Section::Cook).contains(&now.track.as_str()));
    }

    /// **A second device arriving does not restart the room's track.** It believes the
    /// section is silent because it has not been told yet; the room is already playing,
    /// and a start that won a race it did not know it was in would drag everybody back
    /// to the top of a song.
    #[tokio::test]
    async fn a_late_arrival_joins_rather_than_restarting() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap();
        let before = current(&conn, "c", Section::Cook).await.unwrap().unwrap();

        assert!(
            !advance(&conn, "c", Section::Cook, "bob", None, T0 + 30_000)
                .await
                .unwrap(),
            "bob's start conflicts with the room's"
        );
        assert_eq!(
            current(&conn, "c", Section::Cook).await.unwrap().unwrap(),
            before,
            "the room is where it was, mid-track"
        );
    }

    /// **The rollover race: exactly one winner.**
    ///
    /// A track ends on every device within a few milliseconds of the same instant, so
    /// several of them report it. The compare-and-set on the start instant is what makes
    /// "several report, one wins, the rest accept" fall out rather than be arranged —
    /// drop `started_at_ms = ?6` and both calls write, the second overwriting the first,
    /// and the room ends up on whichever track lost the socket race.
    #[tokio::test]
    async fn only_the_first_report_of_a_rollover_moves_the_room() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap();
        let first = current(&conn, "c", Section::Cook).await.unwrap().unwrap();

        // Both heard the same track end; alice's frame reaches the server first.
        assert!(advance(
            &conn,
            "c",
            Section::Cook,
            "alice",
            Some(first.started_at),
            T0 + 180_000
        )
        .await
        .unwrap());
        assert!(
            !advance(
                &conn,
                "c",
                Section::Cook,
                "bob",
                Some(first.started_at),
                T0 + 180_050
            )
            .await
            .unwrap(),
            "bob is answering a state that has already moved"
        );

        let now = current(&conn, "c", Section::Cook).await.unwrap().unwrap();
        assert_eq!(now.started_at, T0 + 180_000, "the winner's instant");
        assert_ne!(now.track, first.track, "and not the track that just ended");
    }

    /// A device that slept through a rollover reports one against a state that is two
    /// tracks old. It is refused — its trouble is its own to heal (#214), and dragging
    /// the room to a new song because one phone woke up late is the reverse of the rule.
    #[tokio::test]
    async fn a_stale_report_moves_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap();
        let first = current(&conn, "c", Section::Cook).await.unwrap().unwrap();
        advance(
            &conn,
            "c",
            Section::Cook,
            "alice",
            Some(first.started_at),
            T0 + 180_000,
        )
        .await
        .unwrap();
        let second = current(&conn, "c", Section::Cook).await.unwrap().unwrap();

        assert!(!advance(
            &conn,
            "c",
            Section::Cook,
            "bob",
            Some(first.started_at),
            T0 + 400_000
        )
        .await
        .unwrap());
        assert_eq!(
            current(&conn, "c", Section::Cook).await.unwrap().unwrap(),
            second
        );
    }

    /// **The room's soundtrack only moves forward.** A rollover stamped at or before the
    /// instant the current track began is refused, so a badly measured clock cannot run
    /// the room's timeline backwards into a position no device can play.
    #[tokio::test]
    async fn a_rollover_that_does_not_move_forward_is_refused() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap();

        assert!(
            !advance(&conn, "c", Section::Cook, "alice", Some(T0), T0)
                .await
                .unwrap(),
            "the same instant is not forward"
        );
        assert!(
            !advance(&conn, "c", Section::Cook, "alice", Some(T0), T0 - 1)
                .await
                .unwrap(),
        );
        assert!(
            advance(&conn, "c", Section::Cook, "alice", Some(T0), T0 + 1)
                .await
                .unwrap()
        );
    }

    /// Sections are independent: `buy`'s music is not `cook`'s, and moving one leaves
    /// the other exactly where the room left it.
    #[tokio::test]
    async fn each_section_holds_its_own_track() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        advance(&conn, "c", Section::Buy, "alice", None, T0)
            .await
            .unwrap();
        advance(&conn, "c", Section::Cook, "alice", None, T0 + 5_000)
            .await
            .unwrap();

        let all = load_all(&conn, "c").await.unwrap();
        assert_eq!(all.len(), 2);
        let buy = current(&conn, "c", Section::Buy).await.unwrap().unwrap();
        let cook = current(&conn, "c", Section::Cook).await.unwrap().unwrap();
        assert!(pool(Section::Buy).contains(&buy.track.as_str()));
        assert!(pool(Section::Cook).contains(&cook.track.as_str()));
        assert_eq!(buy.started_at, T0);
        assert_eq!(cook.started_at, T0 + 5_000);
    }

    /// A section with no tracks writes nothing — silence, honestly, rather than a row
    /// naming a file that does not exist.
    #[tokio::test]
    async fn a_section_with_no_tracks_records_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert!(!advance(&conn, "c", Section::Joy, "alice", None, T0)
            .await
            .unwrap());
        assert!(current(&conn, "c", Section::Joy).await.unwrap().is_none());
        assert!(load_all(&conn, "c").await.unwrap().is_empty());
    }

    /// **The membership predicate, in the write itself.** A signed-in stranger holding
    /// the channel id — which is all a watcher is (#180/#200) — changes no room's music.
    #[tokio::test]
    async fn a_watcher_moves_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert!(!advance(&conn, "c", Section::Cook, "wanda", None, T0)
            .await
            .unwrap());
        assert!(current(&conn, "c", Section::Cook).await.unwrap().is_none());

        advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap();
        let mine = current(&conn, "c", Section::Cook).await.unwrap().unwrap();
        assert!(
            !advance(&conn, "c", Section::Cook, "wanda", Some(T0), T0 + 1_000)
                .await
                .unwrap()
        );
        assert_eq!(
            current(&conn, "c", Section::Cook).await.unwrap().unwrap(),
            mine
        );
    }

    /// **A lobby has music, and that is the one place this guard differs.** People
    /// waiting for the host to start are in the room and hearing it; the plan's *outcome*
    /// is what may not be written before the start, and a soundtrack is not one.
    #[tokio::test]
    async fn a_plan_that_has_not_started_still_has_a_soundtrack() {
        let conn = conn().await;
        lobby(&conn, "c", &["alice"]).await;
        assert!(advance(&conn, "c", Section::Pick, "alice", None, T0)
            .await
            .unwrap());
        assert_eq!(
            current(&conn, "c", Section::Pick)
                .await
                .unwrap()
                .unwrap()
                .track,
            "/pick.mp3"
        );
    }

    /// Rehydrate: the room's whole soundtrack comes back, so a device that reconnects
    /// hours later joins the track in flight rather than starting one of its own.
    #[tokio::test]
    async fn rehydrate_hands_back_every_section_in_flight() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        started_plan(&conn, "d", &["alice"]).await;
        advance(&conn, "c", Section::Buy, "alice", None, T0)
            .await
            .unwrap();
        advance(&conn, "d", Section::Cook, "alice", None, T0)
            .await
            .unwrap();

        let mine = load_all(&conn, "c").await.unwrap();
        assert_eq!(mine.len(), 1, "one plan's music is not another's");
        assert_eq!(mine[0].section, Section::Buy);
        assert_eq!(mine[0].started_at, T0);
    }

    /// A row naming a section this build does not know is skipped rather than guessed
    /// into one it does — the `Section::parse` rule, at the read that would show it.
    #[tokio::test]
    async fn an_unknown_stored_section_is_left_alone() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        conn.execute(
            "INSERT INTO plan_music (channel_id, section, track, started_at_ms)
             VALUES ('c', 'feast', '/music/feast-1.mp3', ?1)",
            libsql::params![T0],
        )
        .await
        .unwrap();
        assert!(load_all(&conn, "c").await.unwrap().is_empty());
    }

    // ---- through the framework ------------------------------------------------

    /// End to end over [`crate::events::ingest`]: the rollover instant is the
    /// initiator's own, normalised through their measured drift, and the room is told
    /// the section's whole state.
    #[tokio::test]
    async fn the_framework_normalises_the_rollover_instant() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let received = T0;
        // Alice's phone is a minute fast, and the track ended 200ms before the frame
        // landed.
        let frames = ingest(
            &conn,
            "c",
            "alice",
            &clock(60_000),
            received - 200 + 60_000,
            received,
            SessionEvent::MusicAdvance {
                section: Section::Cook,
                after: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(frames.len(), 1, "the room is told");
        assert_eq!(
            current(&conn, "c", Section::Cook)
                .await
                .unwrap()
                .unwrap()
                .started_at,
            received - 200,
            "when the track began, not when the frame arrived"
        );
    }

    /// **The loser of the race is told what the winner chose.** The announcement does
    /// not depend on this call having written anything — that is what corrects a device
    /// whose report was refused, instead of leaving it on a track nobody else is playing.
    #[tokio::test]
    async fn a_refused_report_is_answered_with_the_room_s_truth() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        advance(&conn, "c", Section::Cook, "alice", None, T0)
            .await
            .unwrap();
        let winner = current(&conn, "c", Section::Cook).await.unwrap().unwrap();

        let frames = ingest(
            &conn,
            "c",
            "bob",
            &clock(0),
            T0 + 10_000,
            T0 + 10_000,
            SessionEvent::MusicAdvance {
                section: Section::Cook,
                after: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(frames.len(), 1, "bob is answered, not ignored");
        assert_eq!(
            current(&conn, "c", Section::Cook).await.unwrap().unwrap(),
            winner
        );
    }

    /// A watcher's report is refused at the framework's choke point: nothing written,
    /// and **nothing announced**, so no peer's speaker so much as flickers.
    #[tokio::test]
    async fn the_framework_refuses_a_watcher_and_announces_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let frames = ingest(
            &conn,
            "c",
            "wanda",
            &clock(0),
            T0,
            T0,
            SessionEvent::MusicAdvance {
                section: Section::Cook,
                after: None,
            },
        )
        .await
        .unwrap();
        assert!(frames.is_empty(), "silence, as every refusal here is");
        assert!(current(&conn, "c", Section::Cook).await.unwrap().is_none());
    }

    /// **A lobby starts its own music, through the framework.** The direct-write test
    /// above pins the predicate inside the write; this pins the *policy* the choke point
    /// asks first, which is the half where a stricter guard would swallow the report
    /// without a sound and leave a lobby silent for no stated reason.
    #[tokio::test]
    async fn the_framework_lets_a_lobby_start_its_own_music() {
        let conn = conn().await;
        lobby(&conn, "c", &["alice"]).await;
        let frames = ingest(
            &conn,
            "c",
            "alice",
            &clock(0),
            T0,
            T0,
            SessionEvent::MusicAdvance {
                section: Section::Pick,
                after: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(frames.len(), 1, "the lobby is told what it is playing");
        assert_eq!(
            current(&conn, "c", Section::Pick)
                .await
                .unwrap()
                .unwrap()
                .track,
            "/pick.mp3"
        );
    }

    /// A section with no tracks announces nothing at all — there is no state to state.
    #[tokio::test]
    async fn a_section_with_no_tracks_announces_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let frames = ingest(
            &conn,
            "c",
            "alice",
            &clock(0),
            T0,
            T0,
            SessionEvent::MusicAdvance {
                section: Section::Joy,
                after: None,
            },
        )
        .await
        .unwrap();
        assert!(frames.is_empty());
    }

    /// **No track on the wire.** The room owns *what* plays; a frame naming a file is
    /// refused as an unknown field rather than quietly ignored, because every phone in
    /// the plan would load whatever it named.
    #[test]
    fn the_wire_has_nowhere_to_put_a_track() {
        let ok: SessionEvent =
            serde_json::from_str(r#"{"kind":"music_advance","section":"buy","after":null}"#)
                .unwrap();
        assert_eq!(
            ok,
            SessionEvent::MusicAdvance {
                section: Section::Buy,
                after: None
            }
        );
        let wire = serde_json::to_string(&ok).unwrap();
        assert!(!wire.contains("track"), "{wire}");
        assert!(!wire.contains(".mp3"), "{wire}");

        let refused = serde_json::from_str::<SessionEvent>(
            r#"{"kind":"music_advance","section":"buy","after":null,"track":"/evil.mp3"}"#,
        );
        assert!(refused.is_err(), "{refused:?}");

        // And a section nobody has is refused at deserialisation, not defaulted.
        let nowhere = serde_json::from_str::<SessionEvent>(
            r#"{"kind":"music_advance","section":"kitchens","after":null}"#,
        );
        assert!(nowhere.is_err(), "{nowhere:?}");
    }

    /// **The on/off switch stays personal.** Sync decides *what* plays and *where in it
    /// we are*, never whether a given device makes a sound — so neither half of this wire
    /// has anywhere to say so, and no phone can mute, unmute or pause another.
    ///
    /// Asserted as the **whole key set** rather than by hunting for words: a field added
    /// to either frame fails this, which is the only way a check like this stays true of
    /// a field nobody thought to look for.
    #[test]
    fn nothing_on_this_wire_says_whether_a_device_is_audible() {
        let keys = |json: &str| -> std::collections::BTreeSet<String> {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };

        let raised = serde_json::to_string(&SessionEvent::MusicAdvance {
            section: Section::Cook,
            after: Some(T0),
        })
        .unwrap();
        assert_eq!(
            keys(&raised),
            ["after", "kind", "section"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        );

        let announced = serde_json::to_string(&crate::session::ServerMsg::Music {
            section: Section::Cook,
            track: "/music/cook-1.mp3".into(),
            started_at: T0,
        })
        .unwrap();
        assert_eq!(
            keys(&announced),
            ["section", "started_at", "track", "type"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        );

        // And a client that tries to tell the room it is muted is refused rather than
        // having the claim quietly dropped.
        assert!(serde_json::from_str::<SessionEvent>(
            r#"{"kind":"music_advance","section":"cook","after":null,"playing":false}"#
        )
        .is_err());
    }
}
