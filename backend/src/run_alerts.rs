//! Read the `runs` table on a schedule and tell the operator when it says something
//! bad (#183).
//!
//! #174 found that every service path closed its run `completed` unconditionally, so
//! the one column built to answer "did that run work?" answered yes for all 223 rows
//! in production. #182 fixed that: [`crate::runs::Outcome`] is chosen from what the
//! run actually did. This module is the other half — a true record that nobody reads
//! changed nothing. The first night the new vocabulary ran for real, meal-time
//! enrichment recorded 35 `completed`, 12 `failed` and 2 `running`, and nobody was
//! told.
//!
//! ## Pull, not push
//!
//! A writing path could tell somebody at the moment it fails, and that would be
//! immediate. It also cannot cover the case that matters most: a run that dies
//! *without* closing itself — killed by the free tier's 15-minute spin-down, or by a
//! CLI box going to sleep — has no process left to send anything. Those rows (22 of
//! them in production: 9 `ingest`, 7 `enrich`, 5 `enrich_steps`, 1
//! `enrich_equipment`) are exactly the ones nobody can hear from, so whatever we build
//! has to be able to *read* the table rather than only listen to it. Pull also keeps
//! alerting out of the corpus pipeline, which stays dumb.
//!
//! ## Told through the bot, to the one admin
//!
//! Telegram, to `ADMIN_TELEGRAM_USER_ID` ([`crate::auth::admin_chat_id`]) — never a
//! broadcast, and never a new channel. The bot already exists, already holds a private
//! chat with that person (having a session at all means having messaged it), and the
//! admin id is already configured for the health dashboard, so there is no second
//! address to keep in step and no new service to pay for.
//!
//! The case against is real and was weighed: the bot's job today is logins and
//! kitchen invites, and a pipeline alarm landing in the same thread as "what's for
//! dinner" is arguably the wrong room. It loses to the fact that produced this issue.
//! The dashboard is a *correct* signal in a room nobody opens, and one more correct
//! signal in another unopened room is worth nothing. The bot's thread is the one place
//! this person demonstrably reads. If it ever gets noisy the fix is a second bot
//! token, which is configuration rather than redesign.
//!
//! Missing config degrades, it does not crash — the `INGEST_API_KEY` stance (#49) and
//! #146's boot ruling. A deployment with no admin logs the alarm at `error` and
//! answers [`Told::Unconfigured`], so the scheduled job goes red and the operator is
//! still reached, one hop further out.
//!
//! ## Where the schedule comes from
//!
//! `.github/workflows/ingest.yaml` already runs daily and already holds
//! `INGEST_API_KEY`. The check is a second job in it — `needs: sync` so it runs
//! *after* the ingest whose run it might be reporting on, and `if: always()` so an
//! ingest that failed at the HTTP layer still gets the table read. No new
//! infrastructure, and nothing to forget to schedule.
//!
//! Daily is a real limit and an accepted one: a run that dies at 05:00 UTC waits until
//! the next 04:00 to be reported. Telling someone within a day beats the status quo of
//! never, the thresholds below are set so nothing waits *two* cycles, and the cadence
//! is one cron line if a day proves too slow.
//!
//! ## It opens no run of its own
//!
//! Every corpus write carries a `run_id`, and this writes no corpus. A check that
//! opened a run would make the table grow by one row per *read* of it, and would put
//! itself in scope to alarm about — including alarming about the check that died while
//! reporting that something died. It reads, it messages, and it marks; that is all.

use axum::{extract::State, Json};
use libsql::Connection;
use serde::Serialize;

use crate::{auth, error::AppError, runs, AppState};

// ---------------------------------------------------------------------------
// Policy. Everything deciding *whether to wake somebody* is in this one block,
// because a threshold buried at a call site is a policy nobody can find to argue
// with — which is how "who gets told what" stops being a decision and starts being
// an accident.
// ---------------------------------------------------------------------------

/// How often this check actually runs — the `0 4 * * *` cron in
/// `.github/workflows/ingest.yaml`, written down here because
/// [`STALE_AFTER_SECS`] is only defensible relative to it and a threshold whose
/// justification lives in another file is one nobody will re-derive when the cron
/// changes. Nothing reads it at runtime; it is the yardstick [`STALE_AFTER_SECS`] is
/// measured against, just below.
const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// How long a run may sit `running` before it is presumed dead.
///
/// The band the issue argues for is "an hour is ordinary, three days is not". Six
/// hours sits in it, for two reasons that are not taste:
///
/// - It is more than an order of magnitude past the longest run that can legitimately
///   still be open. The daily ingest is capped at 15 minutes by its own caller
///   (`--max-time 900`), the free tier spins the host down at 15 minutes idle anyway,
///   and an enrich batch is minutes. Nothing healthy is alive at six hours.
/// - It is comfortably **shorter than [`CHECK_INTERVAL_SECS`]**. A threshold longer
///   than the interval means a run that crosses it just after a check waits a whole
///   extra cycle, so the reporting delay would be two days rather than one. Keep this
///   below the cron cadence if either ever changes.
const STALE_AFTER_SECS: i64 = 6 * 60 * 60;

/// That relationship, enforced at compile time rather than left to a comment. It is a
/// fact about two constants, so it can be settled before anything runs — a pair that
/// would make every death wait two cycles should not build.
const _: () = assert!(STALE_AFTER_SECS < CHECK_INTERVAL_SECS);

/// How many `failed` runs it takes to speak up. One. A `failed` run is a stage that
/// did not happen — the path returned an error and said so — and there is no weather
/// that produces one.
const FAILED_TO_SPEAK: usize = 1;

/// How many never-closed runs it takes to speak up. One, for the same reason: past
/// [`STALE_AFTER_SECS`] the row is a process that died, and a death is not a rate.
const STALE_TO_SPEAK: usize = 1;

/// How many `partial` runs it takes to speak up.
///
/// **A single `partial` is weather and must never wake anyone.** Sources 502 scrapers
/// most weeks; that is precisely why #174 gave the middle case its own word instead of
/// calling it `failed`. Alerting on one would pin this check to "always firing", which
/// is the same bug as the unconditional `completed` it descends from, wearing an alarm
/// bell.
///
/// Five, because the daily ingest can contribute at most one per window: reaching five
/// means several *different* invocations each dropped work they were handed, which is
/// a pattern rather than a blip. The window is "since the last time we spoke" — a
/// report clears everything it names, so this counts a build-up, not a lifetime total.
///
/// A source that has died *permanently* is a rate over history rather than a count in
/// a window, and is deliberately out of scope: it wants trend data this table cannot
/// answer from three columns.
const PARTIAL_TO_SPEAK: usize = 5;

/// How many run ids a message names before it stops listing them.
///
/// The count is always stated in full; only the enumeration is cut. Telegram caps a
/// message at 4096 characters, and the first check after this ships has a backlog of
/// every historically dead run to get through — around 22 that never closed, plus
/// whatever `failed` has accumulated since #182 — so the untruncated version is both
/// unreadable and, at the extreme, unsendable. Everything counted is marked reported
/// whether or not it was named: the human was told there were twelve.
const MAX_IDS_NAMED: usize = 10;

/// Where the message points for the whole picture: the admin health dashboard, the
/// one reader of this table that already existed.
const DASHBOARD_PATH: &str = "/health";

// ---------------------------------------------------------------------------

/// One run the check has something to say about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Flagged {
    pub id: i64,
    /// `ingest`, `enrich`, `enrich_meal_times`, … — whatever opened it. Worth carrying
    /// because "12 failed" and "12 failed meal-time readings" send an operator to
    /// different places.
    pub kind: String,
    /// How long ago the run opened.
    ///
    /// **Measured entirely by the database's clock**, `unixepoch() - started_at` in
    /// one expression against the row's DB-assigned `started_at`. That is what makes
    /// an age honest here: `0005_runs.sql` made the DB-assigned id the arbiter of
    /// write order precisely because Render and a CLI box disagree about the time, so
    /// an age computed against *this process's* clock would inherit the skew the whole
    /// table was designed around. Reading with one clock at both ends has no skew to
    /// inherit — and #182's ruling only forbids *rewriting* these rows by a wall clock,
    /// not reading them.
    pub age_secs: i64,
}

/// What a scan found, split by what it means. Data only: deciding is
/// [`Scan::worth_telling`] and telling is [`check_with`], so the rule can be argued
/// with without a database and the wording can change without touching the rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Scan {
    /// Closed itself `failed`: a stage returned an error.
    pub failed: Vec<Flagged>,
    /// Closed itself `partial`: it finished, but dropped work it was handed.
    pub partial: Vec<Flagged>,
    /// Never closed, and too old to still be running. The rows no push path could
    /// ever report, because the process that would have sent it is gone.
    pub stale: Vec<Flagged>,
}

/// What became of a message the check decided to send. A closed set rather than a
/// string, so a caller branches on the case and never on the wording of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Telegram accepted it.
    Sent,
    /// There is something to say and no admin configured to say it to.
    Unconfigured,
    /// There is an admin, and the message did not reach them.
    Undelivered,
}

/// What a whole check did — [`Delivery`] plus the case where there was nothing to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Told {
    /// Nothing cleared the thresholds. The overwhelmingly common answer, and the one
    /// that must stay silent — see [`PARTIAL_TO_SPEAK`].
    Quiet,
    Sent,
    Unconfigured,
    Undelivered,
}

impl From<Delivery> for Told {
    fn from(delivery: Delivery) -> Self {
        match delivery {
            Delivery::Sent => Told::Sent,
            Delivery::Unconfigured => Told::Unconfigured,
            Delivery::Undelivered => Told::Undelivered,
        }
    }
}

/// The check's answer to its caller — the scheduled job's log, and what it exits on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub failed: usize,
    pub partial: usize,
    pub stale: usize,
    /// Whether the thresholds were cleared at all, stated separately from [`Self::told`]
    /// so "quiet because nothing was wrong" cannot be confused with "quiet because
    /// nobody could be reached".
    pub alarming: bool,
    pub told: Told,
}

/// Every run this check would speak about, unreported and grouped by what it means.
///
/// One statement per bucket rather than one clever query, because the three predicates
/// are three different questions and reading them separately is how a reviewer can
/// check each one.
pub async fn scan(conn: &Connection) -> anyhow::Result<Scan> {
    Ok(Scan {
        failed: closed_with(conn, runs::FAILED).await?,
        partial: closed_with(conn, runs::PARTIAL).await?,
        stale: never_closed(conn).await?,
    })
}

/// Unreported runs that closed themselves with `status`.
async fn closed_with(conn: &Connection, status: &str) -> anyhow::Result<Vec<Flagged>> {
    flagged(
        conn,
        "SELECT id, kind, unixepoch() - started_at FROM runs
         WHERE status = ?1 AND reported_at IS NULL
         ORDER BY id",
        libsql::params![status],
    )
    .await
}

/// Unreported runs still `running` past [`STALE_AFTER_SECS`].
///
/// The comparison is `>=`, so a run at exactly the threshold counts as dead. An alarm
/// fails safe by speaking: the cost of the inclusive edge is one message about a run
/// that was a second from being reported anyway, and the cost of the exclusive one is
/// a death nobody hears about.
async fn never_closed(conn: &Connection) -> anyhow::Result<Vec<Flagged>> {
    flagged(
        conn,
        "SELECT id, kind, unixepoch() - started_at FROM runs
         WHERE status = ?1 AND reported_at IS NULL
           AND unixepoch() - started_at >= ?2
         ORDER BY id",
        libsql::params![runs::RUNNING, STALE_AFTER_SECS],
    )
    .await
}

async fn flagged(
    conn: &Connection,
    sql: &str,
    params: impl libsql::params::IntoParams,
) -> anyhow::Result<Vec<Flagged>> {
    let mut rows = conn.query(sql, params).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(Flagged {
            id: row.get::<i64>(0)?,
            kind: row.get::<String>(1)?,
            age_secs: row.get::<i64>(2)?,
        });
    }
    Ok(out)
}

impl Scan {
    /// Does this clear the thresholds? The whole policy, in one expression.
    pub fn worth_telling(&self) -> bool {
        self.failed.len() >= FAILED_TO_SPEAK
            || self.stale.len() >= STALE_TO_SPEAK
            || self.partial.len() >= PARTIAL_TO_SPEAK
    }

    /// Every run this scan speaks about, in id order.
    ///
    /// **All three buckets, including the partials that did not clear their own
    /// threshold.** Once a message goes out it names them, so they have been reported,
    /// and marking exactly what the message covers is what keeps `reported_at` meaning
    /// one thing. It also gives [`PARTIAL_TO_SPEAK`] the window it is written against:
    /// a report clears the tally, so five partials means five since we last spoke.
    pub fn ids(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .failed
            .iter()
            .chain(&self.partial)
            .chain(&self.stale)
            .map(|run| run.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// The message, plain text — no `parse_mode`, so a run kind full of underscores
    /// cannot turn into markup or get the whole message rejected.
    pub fn message(&self, site_base_url: &str) -> String {
        let mut out = String::from("recipes: the runs table has something to say.\n");
        for (label, runs) in [
            ("failed", &self.failed),
            ("never closed", &self.stale),
            ("partial", &self.partial),
        ] {
            if !runs.is_empty() {
                out.push('\n');
                out.push_str(&listing(label, runs));
            }
        }
        out.push_str(&format!(
            "\n\n{}{DASHBOARD_PATH}",
            site_base_url.trim_end_matches('/')
        ));
        out
    }
}

/// One bucket as a line: the full count, then up to [`MAX_IDS_NAMED`] of them.
fn listing(label: &str, runs: &[Flagged]) -> String {
    let named = runs
        .iter()
        .take(MAX_IDS_NAMED)
        .map(|run| format!("{} {} ({})", run.id, run.kind, age(run.age_secs)))
        .collect::<Vec<_>>()
        .join(", ");
    let rest = runs.len().saturating_sub(MAX_IDS_NAMED);
    if rest > 0 {
        format!("{label} {}: {named}, and {rest} more", runs.len())
    } else {
        format!("{label} {}: {named}", runs.len())
    }
}

/// A duration at one significant unit — `41d`, `7h`, `12m`. An alarm wants the order
/// of magnitude, and "3714902s" is not one anybody reads at a glance.
fn age(secs: i64) -> String {
    match secs {
        s if s >= 86_400 => format!("{}d", s / 86_400),
        s if s >= 3_600 => format!("{}h", s / 3_600),
        s if s >= 60 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// One check: read the table, apply the policy, tell somebody, and mark **only** what
/// was told.
///
/// `deliver` is the seam. The handler hands it Telegram; a test hands it a closure
/// that answers without a network, which is what makes the ordering rule below
/// something a test can pin rather than something a comment claims.
///
/// **Tell first, mark second, and mark nothing that was not told.** The other order is
/// cheaper and wrong: marking before sending loses the alarm permanently the first
/// time Telegram is having a bad minute, and a lost alarm is the exact bug this whole
/// module exists to end. Failing the other way costs a repeat at the next check, which
/// is noise. Noise beats silence.
///
/// Marking is not part of the same transaction as the send and cannot be — the send is
/// not a database operation. A crash in the gap re-reports at the next check, landing
/// on the tolerable side of the same trade.
pub async fn check_with<D, Fut>(
    conn: &Connection,
    site_base_url: &str,
    deliver: D,
) -> anyhow::Result<CheckReport>
where
    D: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Delivery>,
{
    let scan = scan(conn).await?;
    let report = |told| CheckReport {
        failed: scan.failed.len(),
        partial: scan.partial.len(),
        stale: scan.stale.len(),
        alarming: scan.worth_telling(),
        told,
    };

    if !scan.worth_telling() {
        return Ok(report(Told::Quiet));
    }

    let delivery = deliver(scan.message(site_base_url)).await;
    if delivery == Delivery::Sent {
        runs::mark_reported(conn, &scan.ids()).await?;
    }
    Ok(report(delivery.into()))
}

/// `POST /api/runs/check` — read the runs table and speak up.
///
/// Machine-gated beside `/api/ingest` (`Authorization: Bearer`, `INGEST_API_KEY`): a
/// schedule calls it, not a person, and it is not the operator's *view* — that is
/// `/api/admin/health`, which is session- and admin-gated because a browser reaches it.
///
/// One fresh connection for the whole check rather than [`AppState::with_db`], the way
/// [`crate::ingest`] does it: this is a scheduled job, not a request. A database that
/// will not answer fails the check, which fails the scheduled job, which mails the same
/// human — so the outage reaches them by another road rather than being retried into
/// silence.
pub async fn check(State(state): State<AppState>) -> Result<Json<CheckReport>, AppError> {
    let db = state
        .database
        .connect()
        .map_err(|e| AppError::Internal(format!("run check could not reach the database: {e}")))?;

    let site = state.telegram.frontend_base_url.clone();
    let state_for_send = state.clone();
    let report = check_with(&db, &site, move |message| async move {
        let Some(chat_id) = auth::admin_chat_id(&state_for_send) else {
            // Loud, and still not fatal — the `INGEST_API_KEY` stance (#49). A
            // deployment with no admin configured keeps serving; what it loses is the
            // alarm, and this is the line that says so.
            tracing::error!(
                "runs need reporting and ADMIN_TELEGRAM_USER_ID is not set — nobody can be told. \
                 Set it to the admin's numeric Telegram id (GET /api/me)."
            );
            return Delivery::Unconfigured;
        };
        if auth::send(&state_for_send, chat_id, &message).await {
            Delivery::Sent
        } else {
            tracing::error!("runs need reporting and telegram would not take the message");
            Delivery::Undelivered
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("run check failed: {e:#}")))?;

    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::Outcome;
    use std::cell::RefCell;

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    /// Open a run and close it with `outcome`.
    async fn closed_run(conn: &Connection, kind: &str, outcome: Outcome) -> i64 {
        let id = runs::begin(conn, kind).await.unwrap();
        runs::finish(conn, id, outcome).await.unwrap();
        id
    }

    /// A run that never closed, opened `age_secs` ago.
    ///
    /// The age is backdated in SQL, against the same `unixepoch()` the check reads, so
    /// the fixture and the code under test share the clock the way production's row and
    /// production's query do.
    async fn open_run(conn: &Connection, kind: &str, age_secs: i64) -> i64 {
        let id = runs::begin(conn, kind).await.unwrap();
        conn.execute(
            "UPDATE runs SET started_at = unixepoch() - ?2 WHERE id = ?1",
            libsql::params![id, age_secs],
        )
        .await
        .unwrap();
        id
    }

    /// The site a test's messages link to. A real one, because a fixture that invents
    /// a host renders a link nobody could follow.
    const SITE: &str = "https://recipes.lehlehleh.com";

    /// A delivery seam that records what it was handed and answers however it was told
    /// to — the network, replaced by something a test can assert against.
    struct Fake {
        answer: Delivery,
        seen: RefCell<Vec<String>>,
    }

    impl Fake {
        fn answering(answer: Delivery) -> Self {
            Self {
                answer,
                seen: RefCell::new(Vec::new()),
            }
        }

        async fn check(&self, conn: &Connection) -> CheckReport {
            check_with(conn, SITE, |message| async move {
                self.seen.borrow_mut().push(message);
                self.answer
            })
            .await
            .unwrap()
        }

        fn messages(&self) -> Vec<String> {
            self.seen.borrow().clone()
        }
    }

    /// Whether a run has been marked reported.
    async fn reported(conn: &Connection, id: i64) -> bool {
        let mut rows = conn
            .query(
                "SELECT reported_at FROM runs WHERE id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<Option<i64>>(0)
            .unwrap()
            .is_some()
    }

    /// The status and close stamp a row carries — what must survive being reported.
    async fn record(conn: &Connection, id: i64) -> (String, Option<i64>) {
        let mut rows = conn
            .query(
                "SELECT status, finished_at FROM runs WHERE id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (
            row.get::<String>(0).unwrap(),
            row.get::<Option<i64>>(1).unwrap(),
        )
    }

    /// **The numbers themselves are the policy**, so they are pinned as numbers and not
    /// only as the rule that reads them. Every other test here is written in terms of
    /// the constants and therefore moves with them; without this one, "a single partial
    /// is weather" could quietly become "fifty partials are weather" and nothing would
    /// notice. The threshold-versus-cadence invariant is not here because it does not
    /// need to be: it is a `const _: () = assert!(…)` above, so a pair that would make
    /// every death wait two cycles fails to compile rather than fails a test.
    #[test]
    fn the_thresholds_are_the_ones_that_were_argued_for() {
        assert_eq!(FAILED_TO_SPEAK, 1, "a stage that errored is not weather");
        assert_eq!(STALE_TO_SPEAK, 1, "a death is not a rate");
        assert_eq!(
            PARTIAL_TO_SPEAK, 5,
            "a single partial is weather; five is a pattern"
        );
        assert_eq!(
            STALE_AFTER_SECS,
            6 * 60 * 60,
            "six hours, on a host that spins down at fifteen minutes"
        );
    }

    /// A healthy corpus says nothing. This is the check's normal day, and an alarm
    /// that fires on it is one that gets muted — the same saturation bug as the
    /// unconditional `completed` #174 found, wearing a bell.
    #[tokio::test]
    async fn a_healthy_table_is_silent() {
        let conn = conn().await;
        for _ in 0..40 {
            closed_run(&conn, "ingest", Outcome::Completed).await;
        }
        // Still running, and young enough that this is just a job in progress.
        open_run(&conn, "ingest", STALE_AFTER_SECS - 1).await;

        let fake = Fake::answering(Delivery::Sent);
        let report = fake.check(&conn).await;

        assert_eq!(report.told, Told::Quiet);
        assert!(!report.alarming);
        assert!(
            fake.messages().is_empty(),
            "nobody is messaged about nothing"
        );
    }

    /// **One `failed` speaks.** A stage that returned an error is not weather, and
    /// there is no count to reach first. Kills a threshold raised off 1.
    #[tokio::test]
    async fn a_single_failed_run_is_worth_telling() {
        let conn = conn().await;
        let run = closed_run(&conn, "enrich_meal_times", Outcome::Failed).await;

        let fake = Fake::answering(Delivery::Sent);
        let report = fake.check(&conn).await;

        assert_eq!(report.failed, 1);
        assert_eq!(report.told, Told::Sent);
        assert!(
            fake.messages()[0].contains(&format!("{run} enrich_meal_times")),
            "the message names the run and what it was: {:?}",
            fake.messages()[0]
        );
    }

    /// **A single `partial` is weather.** Sources 502 scrapers most weeks; that is why
    /// `partial` has its own word. Four is still weather, five is a pattern — both
    /// edges pinned, so neither a raised nor a lowered threshold survives.
    #[tokio::test]
    async fn partials_speak_only_once_they_are_a_pattern() {
        let conn = conn().await;
        for _ in 0..PARTIAL_TO_SPEAK - 1 {
            closed_run(&conn, "ingest", Outcome::Partial).await;
        }

        let quiet = Fake::answering(Delivery::Sent);
        let report = quiet.check(&conn).await;
        assert_eq!(report.partial, PARTIAL_TO_SPEAK - 1);
        assert_eq!(
            report.told,
            Told::Quiet,
            "{} partials is still weather",
            PARTIAL_TO_SPEAK - 1
        );
        assert!(quiet.messages().is_empty());

        closed_run(&conn, "ingest", Outcome::Partial).await;
        let loud = Fake::answering(Delivery::Sent);
        let report = loud.check(&conn).await;
        assert_eq!(report.partial, PARTIAL_TO_SPEAK);
        assert_eq!(report.told, Told::Sent, "{PARTIAL_TO_SPEAK} is a pattern");
    }

    /// **The threshold edge for a run that never closed.** One second under is a job in
    /// progress; exactly on it is a death, because an alarm fails safe by speaking.
    /// Kills `>=` flipped to `>`, and either bound moved.
    #[tokio::test]
    async fn a_run_is_dead_at_the_threshold_and_alive_one_second_before() {
        let (alive, dying) = (conn().await, conn().await);

        open_run(&alive, "ingest", STALE_AFTER_SECS - 1).await;
        assert!(
            !scan(&alive).await.unwrap().worth_telling(),
            "a run one second short of the threshold is still running"
        );

        let dead = open_run(&dying, "ingest", STALE_AFTER_SECS).await;
        let found = scan(&dying).await.unwrap();
        assert_eq!(found.stale.iter().map(|r| r.id).collect::<Vec<_>>(), [dead]);
        assert!(found.worth_telling(), "at the threshold it is a death");
    }

    /// The rows no push path could ever report: they are found by *reading* the table,
    /// which is the argument for pull. A `completed` run of the same age is not one of
    /// them — age alone is not the signal.
    #[tokio::test]
    async fn an_old_completed_run_is_not_a_death() {
        let conn = conn().await;
        let old = closed_run(&conn, "ingest", Outcome::Completed).await;
        conn.execute(
            "UPDATE runs SET started_at = unixepoch() - ?2 WHERE id = ?1",
            libsql::params![old, STALE_AFTER_SECS * 100],
        )
        .await
        .unwrap();

        let found = scan(&conn).await.unwrap();
        assert!(found.stale.is_empty(), "it closed; nothing died");
        assert!(!found.worth_telling());
    }

    /// **No re-spam.** The same dead run is reported once and then never again, across
    /// any number of checks — and the state that remembers is in the database, so it
    /// survives the restart that the 15-minute spin-down guarantees.
    #[tokio::test]
    async fn a_reported_run_never_alerts_again() {
        let conn = conn().await;
        let failed = closed_run(&conn, "enrich", Outcome::Failed).await;
        let dead = open_run(&conn, "ingest", STALE_AFTER_SECS * 2).await;

        let first = Fake::answering(Delivery::Sent);
        assert_eq!(first.check(&conn).await.told, Told::Sent);
        assert!(reported(&conn, failed).await);
        assert!(reported(&conn, dead).await);

        // Nothing has changed about the table except that somebody was told.
        let second = Fake::answering(Delivery::Sent);
        let report = second.check(&conn).await;
        assert_eq!(report.told, Told::Quiet);
        assert_eq!((report.failed, report.stale), (0, 0));
        assert!(second.messages().is_empty(), "it does not say it twice");

        // A *new* failure still gets through — silence is per run, not a global mute.
        closed_run(&conn, "enrich", Outcome::Failed).await;
        let third = Fake::answering(Delivery::Sent);
        assert_eq!(third.check(&conn).await.told, Told::Sent);
    }

    /// **Only what was delivered is marked.** Telegram refusing the message must leave
    /// every run unreported, so the next check says it again. Silence is the bug; a
    /// repeat is an annoyance.
    #[tokio::test]
    async fn an_undelivered_alarm_marks_nothing_and_is_retried() {
        let conn = conn().await;
        let failed = closed_run(&conn, "enrich", Outcome::Failed).await;

        let refused = Fake::answering(Delivery::Undelivered);
        let report = refused.check(&conn).await;
        assert_eq!(report.told, Told::Undelivered);
        assert!(report.alarming, "there was something to say");
        assert_eq!(refused.messages().len(), 1, "it was attempted");
        assert!(
            !reported(&conn, failed).await,
            "an unsent alarm is not told"
        );

        let retry = Fake::answering(Delivery::Sent);
        assert_eq!(retry.check(&conn).await.told, Told::Sent);
        assert!(reported(&conn, failed).await);
    }

    /// **Missing config degrades, it does not crash, and it does not lie.** With no
    /// admin the check still answers, still says what it found, and marks nothing — so
    /// configuring an admin later reports the backlog rather than having swallowed it.
    #[tokio::test]
    async fn an_unconfigured_admin_loses_the_message_but_not_the_alarm() {
        let conn = conn().await;
        let failed = closed_run(&conn, "enrich", Outcome::Failed).await;

        let nobody = Fake::answering(Delivery::Unconfigured);
        let report = nobody.check(&conn).await;

        assert_eq!(report.told, Told::Unconfigured);
        assert!(report.alarming);
        assert_eq!(report.failed, 1);
        assert!(
            !reported(&conn, failed).await,
            "nothing was told, so nothing is marked"
        );

        let configured = Fake::answering(Delivery::Sent);
        assert_eq!(configured.check(&conn).await.told, Told::Sent);
    }

    /// **Reporting a run never rewrites what it says happened.** #182 ruled that an
    /// un-closed row must keep saying *nothing closed me*; marking it reported is the
    /// back door that ruling would be lost through, so the row is checked before and
    /// after.
    #[tokio::test]
    async fn the_record_survives_being_reported() {
        let conn = conn().await;
        let dead = open_run(&conn, "ingest", STALE_AFTER_SECS * 10).await;
        let failed = closed_run(&conn, "enrich", Outcome::Failed).await;
        let partial = closed_run(&conn, "ingest", Outcome::Partial).await;
        let before = [
            record(&conn, dead).await,
            record(&conn, failed).await,
            record(&conn, partial).await,
        ];

        assert_eq!(
            Fake::answering(Delivery::Sent).check(&conn).await.told,
            Told::Sent
        );

        assert_eq!(
            [
                record(&conn, dead).await,
                record(&conn, failed).await,
                record(&conn, partial).await,
            ],
            before,
            "status and finished_at are exactly what they were"
        );
        assert_eq!(
            record(&conn, dead).await,
            (runs::RUNNING.to_string(), None),
            "the run nobody closed still says nobody closed it"
        );
    }

    /// A partial that did not clear its own threshold still rides along when something
    /// else speaks — and is marked, because the message named it. That is what makes
    /// [`PARTIAL_TO_SPEAK`] a count since the last report rather than a lifetime total.
    #[tokio::test]
    async fn a_partial_carried_by_another_alarm_is_reported_with_it() {
        let conn = conn().await;
        let partial = closed_run(&conn, "ingest", Outcome::Partial).await;
        closed_run(&conn, "enrich", Outcome::Failed).await;

        let fake = Fake::answering(Delivery::Sent);
        let report = fake.check(&conn).await;

        assert_eq!(report.told, Told::Sent);
        assert_eq!(report.partial, 1);
        assert!(
            fake.messages()[0].contains("partial 1:"),
            "the message says so: {:?}",
            fake.messages()[0]
        );
        assert!(
            reported(&conn, partial).await,
            "it was named, so it was told"
        );
    }

    /// The message states the full count and stops enumerating, so a backlog is
    /// readable and stays inside Telegram's 4096-character limit — and every run it
    /// counted is marked, named or not, or the untold tail would repeat forever.
    #[tokio::test]
    async fn a_backlog_is_counted_in_full_and_listed_in_part() {
        let conn = conn().await;
        let mut dead = Vec::new();
        for _ in 0..MAX_IDS_NAMED + 5 {
            dead.push(open_run(&conn, "ingest", STALE_AFTER_SECS * 3).await);
        }

        let fake = Fake::answering(Delivery::Sent);
        let report = fake.check(&conn).await;
        assert_eq!(report.stale, MAX_IDS_NAMED + 5);

        let message = &fake.messages()[0];
        assert!(
            message.contains(&format!("never closed {}:", MAX_IDS_NAMED + 5)),
            "the count is the whole count: {message:?}"
        );
        assert!(
            message.contains("and 5 more"),
            "and it says what it left out: {message:?}"
        );
        assert!(
            !message.contains(&dead[MAX_IDS_NAMED].to_string()),
            "the eleventh id is not named: {message:?}"
        );
        for id in &dead {
            assert!(reported(&conn, *id).await, "every counted run is marked");
        }
    }

    /// The message points at the dashboard for the whole picture — the reader that
    /// already existed, and the reason this is a nudge rather than a report.
    #[tokio::test]
    async fn the_message_points_at_the_dashboard() {
        let conn = conn().await;
        closed_run(&conn, "enrich", Outcome::Failed).await;
        let scan = scan(&conn).await.unwrap();

        assert!(scan
            .message("https://recipes.lehlehleh.com/")
            .ends_with("https://recipes.lehlehleh.com/health"));
    }

    /// An age is stated at one unit, at each boundary. `3599s` is not a thing anybody
    /// reads at a glance, and a bucket that rounds the wrong way makes a three-day
    /// death look like a three-hour one.
    #[test]
    fn an_age_reads_at_a_glance() {
        assert_eq!(age(0), "0s");
        assert_eq!(age(59), "59s");
        assert_eq!(age(60), "1m");
        assert_eq!(age(3_599), "59m");
        assert_eq!(age(3_600), "1h");
        assert_eq!(age(86_399), "23h");
        assert_eq!(age(86_400), "1d");
        assert_eq!(age(41 * 86_400), "41d");
    }
}
