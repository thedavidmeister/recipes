//! Shared cook timers (#208) — **the first consumer of the session-event framework**
//! ([`crate::events`]).
//!
//! Everyone in a plan is cooking the same pot, so the countdown on it is the plan's,
//! not the phone's. This module is only the *storage* half of that: when an event
//! happened, who raised it and whether they were allowed to are all decided upstream in
//! the framework, and nothing here reads a clock. What it owns is the pair of facts a
//! timer is made of, and the one thing a client is not trusted with.
//!
//! **The initiator owns *when*; the recipe owns *how long*.** [`start`] takes the
//! event's instant from the envelope and reads the step's duration out of the corpus
//! ([`step_seconds`]) — there is no duration on the wire, so one phone cannot make a
//! 30-minute braise three seconds long for the room. The deadline is neither stored nor
//! sent by a client: it is derived from those two facts, in [`load`], and only there.
//!
//! **The row is the timer** (the `buy_checks` rule): starting inserts, dismissing
//! deletes, and *done* is not a state anybody writes — a timer is done when its deadline
//! has passed, which every client can read for itself and which stays true with the
//! process spun down. So a rehydrating client is handed running and finished timers
//! alike, and a timer nobody started simply is not in the list (#208: honest states, no
//! invented data).

use libsql::Connection;
use recipe_core::StructuredStep;
use serde::Serialize;

use crate::session::{not_against_the_decision, seated_in_a_started_plan, Voter};

/// One step's shared countdown, as the room sees it. Mirrors `$lib/session-events`'
/// `RunningTimer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunningTimer {
    /// Which step of the recipe's stored reading (#74) is running.
    pub step: i64,
    /// When it was started, in the **shared timeline** (unix ms) — the initiator's own
    /// instant, corrected for the initiator's clock drift by [`crate::events::normalize`].
    pub started_at: i64,
    /// When it finishes, in the shared timeline: `started_at + seconds * 1000`.
    ///
    /// **Derived here and nowhere else.** The two facts it comes from are both stored
    /// (the tap, and the recipe's duration), so a third stored column could only be a
    /// third thing to disagree with them — the rule #162 applies to per-serving
    /// calories. A client renders `deadline` through its own recorded clock offset; the
    /// countdown it shows is therefore the same one every other phone shows.
    pub deadline: i64,
    /// Who started it. The whole person, like [`crate::session::BuyCheck::by`] — every
    /// surface that shows a timer can say whose pot it is without joining back against
    /// the roster.
    pub started_by: Voter,
}

/// How long the recipe says this step takes, in whole seconds — **the server's own
/// answer**, read from the corpus.
///
/// `None` when the recipe is not in the corpus, its method has not been read into steps
/// yet (`steps` is `[]` until the worker runs — degrade-not-die), the step id is not in
/// that reading, or the step carries no duration. Every one of those is "there is no
/// timer here", and [`start`] writes nothing for it rather than inventing a length: a
/// step the corpus does not time is a step the room does not count down (#158 — a
/// missing duration is a gap in our reading, never a zero).
///
/// The reading is parsed rather than queried into, because `recipes.steps` is a JSON
/// document — the same shape `derive` wrote and the same shape the browser renders, so
/// there is one description of a step and this cannot drift from it. A malformed
/// document answers `None`, which reads as "no timer" rather than taking the process
/// down.
pub async fn step_seconds(
    conn: &Connection,
    source: &str,
    id: &str,
    step: i64,
) -> anyhow::Result<Option<i64>> {
    let mut rows = conn
        .query(
            "SELECT steps FROM recipes WHERE source = ?1 AND id = ?2 LIMIT 1",
            libsql::params![source, id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let raw: String = row.get(0)?;
    let Ok(steps) = serde_json::from_str::<Vec<StructuredStep>>(&raw) else {
        return Ok(None);
    };
    Ok(steps
        .iter()
        .find(|s| i64::from(s.id) == step)
        .and_then(|s| s.seconds)
        .map(i64::from))
}

/// Start (or restart) one step's shared countdown, reporting whether it was written.
///
/// `at` is the event's instant on the shared timeline, from the envelope — this
/// function does not read a clock, and that is the point: the tap is the event, not the
/// arrival of the frame carrying it.
///
/// Restarting is a plain overwrite. The primary key does not include the person, so a
/// second cook tapping Start moves the one countdown rather than adding a second one
/// for the same pot — last writer wins, the way `buy_checks` hands a line over.
///
/// The roster, the start and the recipe are all in the insert's own predicate
/// ([`seated_in_a_started_plan`], [`not_against_the_decision`]), not merely in the
/// framework's preceding check: a seat can be given up, and a plan can decide, in the
/// round trip between the two. The recipe guard is what keeps a timer on the *plan's*
/// meal — the step id means nothing except against a particular recipe's reading, so a
/// client naming another one would be counting down a pot this kitchen is not cooking.
pub async fn start(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    step: i64,
    user: &str,
    at: i64,
) -> anyhow::Result<bool> {
    let Some(seconds) = step_seconds(conn, source, id, step).await? else {
        return Ok(false);
    };
    let written = conn
        .execute(
            &format!(
                "INSERT INTO plan_timers
                    (channel_id, source, id, step_id, started_at_ms, seconds, user_id)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE {} AND {}
                 ON CONFLICT(channel_id, source, id, step_id) DO UPDATE SET
                    started_at_ms = excluded.started_at_ms,
                    seconds = excluded.seconds,
                    user_id = excluded.user_id",
                seated_in_a_started_plan("?7"),
                not_against_the_decision("?2", "?3")
            ),
            libsql::params![channel, source, id, step, at, seconds, user],
        )
        .await?;
    Ok(written > 0)
}

/// Take one step's countdown off the room's screens — the row is the timer, so this is
/// a delete.
///
/// Deleting nothing is success: dismissing a timer somebody else already dismissed is
/// the state the caller asked for. Carries the same predicates the start does, and needs
/// to know *who* is asking even though the row it removes may record somebody else —
/// anyone cooking this meal may stop any of its timers, and nobody else may. A guarded
/// claim with an unguarded release is not guarded (`session::untick_item` says the same
/// thing about a shopping line).
pub async fn dismiss(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    step: i64,
    user: &str,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "DELETE FROM plan_timers
              WHERE channel_id = ?1 AND source = ?2 AND id = ?3 AND step_id = ?4
                AND {} AND {}",
            seated_in_a_started_plan("?5"),
            not_against_the_decision("?2", "?3")
        ),
        libsql::params![channel, source, id, step, user],
    )
    .await?;
    Ok(())
}

/// Every timer this plan has running or finished on this recipe, in step order.
///
/// **Finished ones are included**, and that is not an oversight: a timer whose deadline
/// passed while everybody's browser was closed is a pot that needs taking off the heat,
/// and it is the state a rehydrating client most needs to be told about. Dismissing is
/// what removes a row; time passing is not.
pub async fn load(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
) -> anyhow::Result<Vec<RunningTimer>> {
    let mut rows = conn
        .query(
            "SELECT t.step_id, t.started_at_ms, t.seconds, t.user_id, u.username
             FROM plan_timers t
             LEFT JOIN users u ON u.telegram_user_id = t.user_id
             WHERE t.channel_id = ?1 AND t.source = ?2 AND t.id = ?3
             ORDER BY t.step_id",
            libsql::params![channel, source, id],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let step: i64 = row.get(0)?;
        let started_at: i64 = row.get(1)?;
        let seconds: i64 = row.get(2)?;
        out.push(RunningTimer {
            step,
            started_at,
            // The one place the deadline exists. See `RunningTimer::deadline`.
            deadline: started_at + seconds * 1000,
            started_by: Voter {
                telegram_user_id: row.get(3)?,
                username: row.get::<Option<String>>(4)?,
            },
        });
    }
    Ok(out)
}

/// Every timer this plan holds, grouped by the recipe it is on — the rehydrate read.
///
/// By plan rather than by the plan's decision, so a (re)joining client is told about
/// every pot that is on and is never left holding a countdown nobody mentioned. The
/// cook screen ignores a group for a recipe it is not cooking, exactly as `buy` ignores
/// another recipe's checklist.
///
/// Ordered by recipe then step, so the grouping is a single pass over consecutive rows
/// rather than a map the caller has to sort back into a stable order.
pub async fn load_all(
    conn: &Connection,
    channel: &str,
) -> anyhow::Result<Vec<(String, String, Vec<RunningTimer>)>> {
    let mut rows = conn
        .query(
            "SELECT t.source, t.id, t.step_id, t.started_at_ms, t.seconds, t.user_id, u.username
             FROM plan_timers t
             LEFT JOIN users u ON u.telegram_user_id = t.user_id
             WHERE t.channel_id = ?1
             ORDER BY t.source, t.id, t.step_id",
            libsql::params![channel],
        )
        .await?;
    let mut out: Vec<(String, String, Vec<RunningTimer>)> = Vec::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let step: i64 = row.get(2)?;
        let started_at: i64 = row.get(3)?;
        let seconds: i64 = row.get(4)?;
        let timer = RunningTimer {
            step,
            started_at,
            deadline: started_at + seconds * 1000,
            started_by: Voter {
                telegram_user_id: row.get(5)?,
                username: row.get::<Option<String>>(6)?,
            },
        };
        match out.last_mut() {
            Some((s, i, group)) if *s == source && *i == id => group.push(timer),
            _ => out.push((source, id, vec![timer])),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ingest, ClockOffset, SessionEvent};
    use crate::session::test_support::{decide, started_plan};
    use crate::session::ServerMsg;

    /// Chicken Handi's real stored reading is the fixture the frontend's stories use;
    /// what matters here is only that a step carries a duration and another does not.
    const STEPS: &str = r#"[
        {"id":1,"text":"chop the onion","kind":"prep","seconds":120,"after":[]},
        {"id":6,"text":"fry the garlic","kind":"cook","seconds":60,"after":[1]},
        {"id":7,"text":"cook the tomatoes","kind":"cook","seconds":300,"after":[6]},
        {"id":9,"text":"season to taste","kind":"cook","seconds":null,"after":[7]}
    ]"#;

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn.execute(
            "INSERT INTO recipes (source, id, title, instructions, ingredients, steps)
             VALUES ('themealdb', '52795', 'Chicken Handi', 'cook it', '[]', ?1)",
            libsql::params![STEPS],
        )
        .await
        .unwrap();
        conn
    }

    /// A connection whose clock has been measured as `offset` ms ahead of the server.
    fn clock(offset: i64) -> ClockOffset {
        let mut c = ClockOffset::new();
        c.ping_sent(1_000);
        assert!(c.pong(1_000, 1_020 + offset, 1_040));
        c
    }

    /// The duration comes out of the corpus, per step — including the honest `None` for
    /// a step the reading left untimed (#158).
    #[tokio::test]
    async fn durations_are_read_from_the_recipe() {
        let conn = conn().await;
        assert_eq!(
            step_seconds(&conn, "themealdb", "52795", 7).await.unwrap(),
            Some(300)
        );
        assert_eq!(
            step_seconds(&conn, "themealdb", "52795", 1).await.unwrap(),
            Some(120)
        );
        assert_eq!(
            step_seconds(&conn, "themealdb", "52795", 9).await.unwrap(),
            None,
            "a step the corpus does not time"
        );
        assert_eq!(
            step_seconds(&conn, "themealdb", "52795", 404)
                .await
                .unwrap(),
            None,
            "a step that is not in the reading"
        );
        assert_eq!(
            step_seconds(&conn, "themealdb", "nope", 7).await.unwrap(),
            None,
            "a recipe that is not in the corpus"
        );
    }

    /// **The deadline is the recipe's duration on the initiator's instant** — and the
    /// client says neither. This is the mutation target the whole design turns on: swap
    /// the stored `seconds` for anything a frame could carry, or the envelope's `at` for
    /// a receipt, and this fails.
    #[tokio::test]
    async fn the_deadline_is_the_tap_plus_the_corpus_duration() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let tap = 1_700_000_000_000;

        assert!(start(&conn, "c", "themealdb", "52795", 7, "alice", tap)
            .await
            .unwrap());

        let timers = load(&conn, "c", "themealdb", "52795").await.unwrap();
        assert_eq!(timers.len(), 1);
        assert_eq!(timers[0].step, 7);
        assert_eq!(timers[0].started_at, tap);
        assert_eq!(
            timers[0].deadline,
            tap + 300 * 1000,
            "five minutes, per the corpus"
        );
        assert_eq!(timers[0].started_by.telegram_user_id, "alice");
    }

    /// A step with no duration in the corpus starts no timer, rather than one of zero
    /// length that is instantly done.
    #[tokio::test]
    async fn an_untimed_step_starts_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert!(!start(&conn, "c", "themealdb", "52795", 9, "alice", 1_000)
            .await
            .unwrap());
        assert!(load(&conn, "c", "themealdb", "52795")
            .await
            .unwrap()
            .is_empty());
    }

    /// One pot, one countdown: a second cook tapping Start moves the timer instead of
    /// adding a second one, and the row records who moved it.
    #[tokio::test]
    async fn restarting_moves_the_one_timer() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice", "bob"]).await;
        assert!(start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap());
        assert!(start(&conn, "c", "themealdb", "52795", 7, "bob", 9_000)
            .await
            .unwrap());

        let timers = load(&conn, "c", "themealdb", "52795").await.unwrap();
        assert_eq!(timers.len(), 1, "one pot, one countdown");
        assert_eq!(timers[0].started_at, 9_000);
        assert_eq!(timers[0].started_by.telegram_user_id, "bob");
    }

    /// Dismissing is a delete, and dismissing what is not there is success.
    #[tokio::test]
    async fn dismissing_removes_the_row() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap();
        dismiss(&conn, "c", "themealdb", "52795", 7, "alice")
            .await
            .unwrap();
        assert!(load(&conn, "c", "themealdb", "52795")
            .await
            .unwrap()
            .is_empty());
        dismiss(&conn, "c", "themealdb", "52795", 7, "alice")
            .await
            .unwrap();
    }

    /// A finished timer is still a timer: rehydration carries it, because a pot whose
    /// time is up while every browser was closed is exactly what somebody needs telling.
    #[tokio::test]
    async fn a_finished_timer_is_still_in_the_list() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        // Started long ago: 300s after this is still far in the past.
        start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap();
        start(&conn, "c", "themealdb", "52795", 6, "alice", i64::MAX / 4)
            .await
            .unwrap();

        let timers = load(&conn, "c", "themealdb", "52795").await.unwrap();
        assert_eq!(timers.len(), 2, "the finished one and the running one");
        assert_eq!(timers[0].step, 6);
        assert_eq!(timers[1].step, 7);
    }

    /// **The membership guard, in the write itself.** A signed-in stranger holding the
    /// channel id — which is all a watcher is (#180/#200) — starts nothing.
    #[tokio::test]
    async fn a_non_member_starts_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        assert!(
            !start(&conn, "c", "themealdb", "52795", 7, "mallory", 1_000)
                .await
                .unwrap()
        );
        assert!(load(&conn, "c", "themealdb", "52795")
            .await
            .unwrap()
            .is_empty());
    }

    /// And cannot stop one either — a guarded claim with an unguarded release is not
    /// guarded.
    #[tokio::test]
    async fn a_non_member_dismisses_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap();
        dismiss(&conn, "c", "themealdb", "52795", 7, "mallory")
            .await
            .unwrap();
        assert_eq!(
            load(&conn, "c", "themealdb", "52795").await.unwrap().len(),
            1,
            "the timer is still running"
        );
    }

    /// A lobby that has not started cooks nothing.
    #[tokio::test]
    async fn a_plan_that_has_not_started_has_no_timers() {
        let conn = conn().await;
        crate::session::test_support::lobby(&conn, "c", &["alice"]).await;
        assert!(!start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap());
    }

    /// A timer belongs to the meal the plan decided: a member naming a different recipe
    /// writes nothing (#201's guard, on this table too).
    #[tokio::test]
    async fn a_timer_on_a_recipe_the_plan_did_not_decide_is_refused() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        decide(&conn, "c", "themealdb", "52874").await;
        assert!(!start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap());
    }

    // ---- through the framework ------------------------------------------------

    /// End to end over [`crate::events::ingest`]: a member's tap is normalised through
    /// their own recorded clock offset, and the room is told the whole timer list.
    #[tokio::test]
    async fn the_framework_normalises_the_initiators_tap() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let received = 1_700_000_000_000;
        // Alice's phone is a minute fast, and she tapped 200ms before the frame landed.
        let offset = clock(60_000);
        let frames = ingest(
            &conn,
            "c",
            "alice",
            &offset,
            received - 200 + 60_000,
            received,
            SessionEvent::TimerStart {
                source: "themealdb".into(),
                id: "52795".into(),
                step: 7,
            },
        )
        .await
        .unwrap();

        assert_eq!(frames.room.len(), 1, "the room is told");
        assert!(
            frames.initiator.is_empty(),
            "and an accepted event answers its own device nothing beyond that"
        );
        let timers = load(&conn, "c", "themealdb", "52795").await.unwrap();
        assert_eq!(
            timers[0].started_at,
            received - 200,
            "her tap, not her clock"
        );
        assert_eq!(timers[0].deadline, received - 200 + 300_000);
    }

    /// A watcher's event is refused at the framework's choke point: nothing written, and
    /// nothing announced — so no peer's screen so much as flickers.
    ///
    /// Their **own** screen is answered, with the plan's timers for that recipe as they
    /// actually are (#222). Empty here, and that emptiness is the point: it is the frame
    /// that takes a would-be countdown off the refused device rather than leaving one
    /// running against a deadline nobody recorded.
    #[tokio::test]
    async fn the_framework_refuses_a_watcher_and_announces_nothing() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        let frames = ingest(
            &conn,
            "c",
            "wanda",
            &clock(0),
            1_700_000_000_000,
            1_700_000_000_000,
            SessionEvent::TimerStart {
                source: "themealdb".into(),
                id: "52795".into(),
                step: 7,
            },
        )
        .await
        .unwrap();
        assert!(
            frames.room.is_empty(),
            "nothing happened, so the room hears nothing"
        );
        match frames.initiator.as_slice() {
            [ServerMsg::Timers { source, id, timers }] => {
                assert_eq!((source.as_str(), id.as_str()), ("themealdb", "52795"));
                assert!(timers.is_empty(), "and the truth is that no pot is on");
            }
            other => panic!("the refused device was told {other:?}"),
        }
        assert!(load(&conn, "c", "themealdb", "52795")
            .await
            .unwrap()
            .is_empty());
    }

    /// And cannot dismiss a running one — nor take one off their own screen by being
    /// refused: the answer is the pot that **is** on, not an empty list (#222).
    #[tokio::test]
    async fn the_framework_refuses_a_watchers_dismiss() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        start(&conn, "c", "themealdb", "52795", 7, "alice", 1_000)
            .await
            .unwrap();
        let frames = ingest(
            &conn,
            "c",
            "wanda",
            &clock(0),
            1_700_000_000_000,
            1_700_000_000_000,
            SessionEvent::TimerDismiss {
                source: "themealdb".into(),
                id: "52795".into(),
                step: 7,
            },
        )
        .await
        .unwrap();
        assert!(frames.room.is_empty(), "the room hears nothing");
        match frames.initiator.as_slice() {
            [ServerMsg::Timers { timers, .. }] => assert_eq!(
                timers.len(),
                1,
                "and the refused device is told the timer is still running"
            ),
            other => panic!("the refused device was told {other:?}"),
        }
        assert_eq!(
            load(&conn, "c", "themealdb", "52795").await.unwrap().len(),
            1
        );
    }

    /// **Receipt latency does not move the deadline**, through the whole path: the same
    /// tap delivered promptly and delivered two seconds late produces the same deadline.
    #[tokio::test]
    async fn a_late_frame_does_not_shorten_the_timer() {
        let conn = conn().await;
        started_plan(&conn, "c", &["alice"]).await;
        started_plan(&conn, "d", &["alice"]).await;
        let tap = 1_700_000_000_000;
        let event = SessionEvent::TimerStart {
            source: "themealdb".into(),
            id: "52795".into(),
            step: 7,
        };

        ingest(&conn, "c", "alice", &clock(0), tap, tap + 30, event.clone())
            .await
            .unwrap();
        ingest(&conn, "d", "alice", &clock(0), tap, tap + 2_000, event)
            .await
            .unwrap();

        let prompt = load(&conn, "c", "themealdb", "52795").await.unwrap();
        let late = load(&conn, "d", "themealdb", "52795").await.unwrap();
        assert_eq!(prompt[0].deadline, late[0].deadline);
        assert_eq!(prompt[0].deadline, tap + 300_000);
    }
}
