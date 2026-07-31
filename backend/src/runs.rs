//! Run generation: a monotonic id per pipeline invocation (#11 write-path
//! hardening).
//!
//! Every invocation — the `/api/ingest` pipeline, or a CLI `enrich`/`derive` —
//! opens a run here at start and closes it at end. The DB-assigned `id` is a
//! total order free of the clock skew between Render and a CLI box, so corpus
//! writes stamp it and guard on it (only an equal-or-newer run overwrites) and no
//! concurrent or partial run clobbers another. The row is also the audit trail: a
//! run still `running` long after `started_at` is one that died mid-flight.
//!
//! **The status is what the run did, not what the caller wanted (#174).** Every
//! service path used to close its run [`COMPLETED`] unconditionally, so the one
//! column built to answer "did that run work?" answered "yes" for all 223 rows in
//! production and `failed` had never been written in the database's lifetime. The
//! outcome now comes from the run — see [`Outcome`] for the three states and the
//! rule that picks between them.
//!
//! **An un-closed run is not swept to a terminal status, deliberately.** Production
//! holds 22 rows with `finished_at IS NULL` (9 `ingest`, 7 `enrich`, 5
//! `enrich_steps`, 1 `enrich_equipment`), and the free tier spins down at 15
//! minutes, so a process dying mid-ingest is ordinary rather than exceptional.
//! Rewriting those rows would destroy the one thing they say — *nothing ever closed
//! me* — which is a different fact from a run that closed itself and reported
//! failure. It would also need an age threshold, i.e. a wall clock, and the clock
//! skew between Render and a CLI box is precisely why `0005_runs.sql` made the
//! DB-assigned id the arbiter in the first place; there is no honest clock here to
//! sweep by. What a path that *knows* it failed must do instead is close its own
//! run [`Outcome::Failed`] ([`fail`]), so `running` keeps meaning only the one
//! thing.
//!
//! **Being read is a separate fact from what happened (#183).** A true record that
//! nobody looks at changed nothing, so [`crate::run_alerts`] reads this table on a
//! schedule and messages the operator. What it writes back is [`mark_reported`], and
//! it writes *only* that: `reported_at` says somebody was told, never that the run
//! went differently. The status column is still owned by [`finish`] alone, which is
//! what keeps the paragraph above true — a row's account of itself survives being
//! complained about.
//!
//! The vocabulary is not a `CHECK` constraint. Every write goes through [`finish`],
//! which takes an [`Outcome`] rather than a string, so the type is the constraint at
//! the only writer — and a rebuild of this table to bolt one on would buy nothing
//! the enum does not already give.

use libsql::Connection;

/// A run's terminal state, as written to `runs.status`. See [`Outcome`], which is
/// what callers pass; these are the strings it maps to.
pub const COMPLETED: &str = "completed";
pub const PARTIAL: &str = "partial";
pub const FAILED: &str = "failed";
/// The status a run carries until something closes it. Not an [`Outcome`]: it is the
/// absence of one, and a path can never choose it — [`begin`] is the only writer.
pub const RUNNING: &str = "running";

/// How a run ended, in severity order (#174).
///
/// Three terminal states, not two, because two cannot say what happened. A trigger
/// that fetched nothing and derived nothing must not read the same as one that did
/// the whole job — but a scheduled ingest meets a dead source most weeks, and
/// calling that `failed` would pin the column to a single value all over again: the
/// same bug wearing a different constant. So the middle case gets its own word.
///
/// - [`Completed`](Outcome::Completed) — the run did everything it set out to do.
///   Nothing it was handed was dropped.
/// - [`Partial`](Outcome::Partial) — the run finished, but dropped work it was
///   handed. The detail is in the run's report: `sync`'s per-URL failures, a push's
///   rejections. The status says *go look*; the report says how bad.
/// - [`Failed`](Outcome::Failed) — a stage did not happen. It returned an error.
///
/// **Severity is not thresholded on a count.** "Fetched 0 of 103" and "fetched 100
/// of 103" are both [`Partial`](Outcome::Partial); picking a cutoff at which one
/// becomes [`Failed`](Outcome::Failed) would be inventing policy at the call site,
/// and the count is already in the report for anyone who wants it.
///
/// **A standing condition of the corpus is not a run outcome.** `derive` counts
/// payloads no adapter normalizes today as `skipped`; that is a property of what we
/// hold, identical on every run until an adapter is fixed, so folding it in here
/// would saturate the column exactly the way an unconditional `COMPLETED` does.
///
/// A run made of several stages records the **worst** of them ([`Outcome::worst`]);
/// the derived `Ord` is that severity order, lowest to highest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    #[default]
    Completed,
    Partial,
    Failed,
}

impl Outcome {
    /// The `runs.status` string for this outcome.
    pub fn status(self) -> &'static str {
        match self {
            Outcome::Completed => COMPLETED,
            Outcome::Partial => PARTIAL,
            Outcome::Failed => FAILED,
        }
    }

    /// The worse of two stages' outcomes — how a multi-stage run reports itself. A
    /// clean derive does not redeem a sync that dropped half its catalog.
    pub fn worst(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Open a run of `kind` (`"ingest"` | `"enrich"` | `"derive"` | `"refresh"`) and
/// return its monotonic id. Uses `INSERT … RETURNING` so the id comes back with
/// the insert — race-free, unlike `last_insert_rowid` on a shared connection.
pub async fn begin(conn: &Connection, kind: &str) -> anyhow::Result<i64> {
    let mut rows = conn
        .query(
            "INSERT INTO runs (kind) VALUES (?1) RETURNING id",
            libsql::params![kind],
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("INSERT … RETURNING gave no row"))?;
    Ok(row.get::<i64>(0)?)
}

/// Close a run: stamp `finished_at` and the [`Outcome`] it ended in. Leaving a run
/// un-closed is what marks it as died-mid-flight, so a path that reaches an end —
/// any end — closes its own run.
pub async fn finish(conn: &Connection, run_id: i64, outcome: Outcome) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE runs SET finished_at = unixepoch(), status = ?2 WHERE id = ?1",
        libsql::params![run_id, outcome.status()],
    )
    .await?;
    Ok(())
}

/// Close `run_id` as [`Outcome::Failed`] and hand `error` straight back — for a
/// caller about to propagate it, which reads `return Err(runs::fail(conn, run, e))`.
///
/// A path that bails mid-run used to leave its row open, which put a request that
/// errored into the same bucket as a container Render killed. Those are different
/// facts and the table has a word for each; this is how a path that knows it failed
/// says so.
///
/// Closing is best-effort on purpose: the error being propagated is the caller's
/// news, and losing it to a second database failure — likely the same one — would
/// replace a real diagnosis with a bookkeeping complaint. A close that fails leaves
/// the row `running`, which is exactly what it was going to be anyway.
pub async fn fail(conn: &Connection, run_id: i64, error: anyhow::Error) -> anyhow::Error {
    if let Err(e) = finish(conn, run_id, Outcome::Failed).await {
        tracing::warn!("could not close failed run {run_id}: {e}");
    }
    error
}

/// Record that somebody has been told about these runs (#183), so the next check does
/// not say it again.
///
/// **It writes `reported_at` and nothing else.** Not `status`, not `finished_at`: what
/// the row says happened is the record, and a run being complained about does not
/// change it. That is the same line #182 drew when it refused to sweep un-closed rows —
/// the difference between *nothing closed me* and *something closed me badly* is worth
/// more than a tidy column, and there is no honest wall clock to sweep by anyway. This
/// adds a fact about us beside the run's account of itself; it does not edit the
/// account.
///
/// Idempotent, so it is safe to re-run: setting `reported_at` twice reaches the same
/// state, and the alarm would rather repeat itself than lose a row's alarm.
///
/// Lives here rather than in [`crate::run_alerts`] because this table has one writing
/// module, and keeping it that way is what makes "the status is only ever written by
/// [`finish`]" a claim you can check by reading one file.
pub async fn mark_reported(conn: &Connection, run_ids: &[i64]) -> anyhow::Result<()> {
    if run_ids.is_empty() {
        return Ok(());
    }
    // The id list is built rather than fixed, so the placeholders have to be too. They
    // are bound, not interpolated: the ids came out of this table as `i64`, so there is
    // nothing here a string could smuggle, and binding them keeps it that way if the
    // caller ever changes.
    let placeholders = (1..=run_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute(
        &format!("UPDATE runs SET reported_at = unixepoch() WHERE id IN ({placeholders})"),
        libsql::params_from_iter(run_ids.iter().copied()),
    )
    .await?;
    Ok(())
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

    /// Ids are monotonic (a total order) and a fresh run is `running` until closed.
    #[tokio::test]
    async fn runs_are_monotonic_and_track_status() {
        let conn = conn().await;
        let first = begin(&conn, "ingest").await.unwrap();
        let second = begin(&conn, "enrich").await.unwrap();
        assert!(second > first, "ids must be monotonic: {second} > {first}");

        let mut rows = conn
            .query(
                "SELECT status, finished_at FROM runs WHERE id = ?1",
                libsql::params![first],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "running");
        assert!(row.get::<Option<i64>>(1).unwrap().is_none());

        finish(&conn, first, Outcome::Completed).await.unwrap();
        let (status, finished) = closed(&conn, first).await;
        assert_eq!(status, "completed");
        assert!(finished.is_some());
    }

    /// The status a run row carries, and whether it was closed.
    async fn closed(conn: &Connection, run_id: i64) -> (String, Option<i64>) {
        let mut rows = conn
            .query(
                "SELECT status, finished_at FROM runs WHERE id = ?1",
                libsql::params![run_id],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (
            row.get::<String>(0).unwrap(),
            row.get::<Option<i64>>(1).unwrap(),
        )
    }

    /// Each outcome reaches the row as its own word — the point of #174 is that the
    /// column can hold more than one answer.
    #[tokio::test]
    async fn every_outcome_writes_its_own_status() {
        let conn = conn().await;
        for (outcome, expected) in [
            (Outcome::Completed, "completed"),
            (Outcome::Partial, "partial"),
            (Outcome::Failed, "failed"),
        ] {
            let run = begin(&conn, "ingest").await.unwrap();
            finish(&conn, run, outcome).await.unwrap();
            let (status, finished) = closed(&conn, run).await;
            assert_eq!(status, expected, "{outcome:?} must record as {expected}");
            assert!(finished.is_some(), "a closed run is stamped");
        }
    }

    /// `worst` is the severity order, and it is what a multi-stage run reports: a
    /// clean stage never redeems a broken one, in either argument order.
    #[test]
    fn worst_takes_the_worse_stage() {
        use Outcome::{Completed, Failed, Partial};
        assert!(Completed < Partial && Partial < Failed, "severity order");
        for (a, b, expected) in [
            (Completed, Completed, Completed),
            (Completed, Partial, Partial),
            (Completed, Failed, Failed),
            (Partial, Failed, Failed),
            (Partial, Partial, Partial),
            (Failed, Failed, Failed),
        ] {
            assert_eq!(a.worst(b), expected, "{a:?}.worst({b:?})");
            assert_eq!(b.worst(a), expected, "worst is symmetric");
        }
    }

    /// `fail` closes the run as failed and hands the error back untouched — a run
    /// that errored says `failed`, and is not abandoned to look like a process death.
    #[tokio::test]
    async fn fail_closes_the_run_and_returns_the_error() {
        let conn = conn().await;
        let run = begin(&conn, "enrich").await.unwrap();
        let err = fail(&conn, run, anyhow::anyhow!("turso truncated the response")).await;
        assert_eq!(err.to_string(), "turso truncated the response");
        let (status, finished) = closed(&conn, run).await;
        assert_eq!(status, "failed");
        assert!(finished.is_some());
    }

    /// Whether a run has been complained about.
    async fn reported(conn: &Connection, run_id: i64) -> Option<i64> {
        let mut rows = conn
            .query(
                "SELECT reported_at FROM runs WHERE id = ?1",
                libsql::params![run_id],
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<Option<i64>>(0)
            .unwrap()
    }

    /// **The record survives being complained about (#183).** `mark_reported` adds a
    /// fact about us; it does not edit the run's account of itself. The un-closed row
    /// is the one that matters — #182 ruled it must never be swept to a terminal
    /// status, and marking it reported is exactly the back door that ruling would be
    /// lost through.
    #[tokio::test]
    async fn marking_reported_never_rewrites_what_the_row_says() {
        let conn = conn().await;
        let died = begin(&conn, "ingest").await.unwrap();
        let errored = begin(&conn, "enrich").await.unwrap();
        finish(&conn, errored, Outcome::Failed).await.unwrap();
        let before = closed(&conn, errored).await;

        mark_reported(&conn, &[died, errored]).await.unwrap();

        // The run nobody closed still says nothing closed it.
        assert_eq!(
            closed(&conn, died).await,
            (RUNNING.to_string(), None),
            "a reported run that never closed is still `running` with no finished_at"
        );
        // And the one that closed itself still says exactly what it said.
        assert_eq!(
            closed(&conn, errored).await,
            before,
            "reporting a failed run changes neither its status nor when it closed"
        );
        assert!(reported(&conn, died).await.is_some());
        assert!(reported(&conn, errored).await.is_some());
    }

    /// It marks the runs it was given and no others — the neighbouring row is what a
    /// too-broad `WHERE` would take with it, and a wrongly-marked run is one nobody
    /// will ever be told about.
    #[tokio::test]
    async fn marking_reported_touches_only_the_named_runs() {
        let conn = conn().await;
        let named = begin(&conn, "ingest").await.unwrap();
        let bystander = begin(&conn, "enrich").await.unwrap();

        mark_reported(&conn, &[named]).await.unwrap();

        assert!(reported(&conn, named).await.is_some());
        assert!(
            reported(&conn, bystander).await.is_none(),
            "a run nobody was told about is still unreported"
        );
    }

    /// Marking nothing is a no-op rather than a malformed `IN ()`, and marking twice
    /// reaches the same state — the alarm re-runs on a schedule, so both are ordinary.
    #[tokio::test]
    async fn marking_reported_is_safe_to_repeat_and_safe_to_skip() {
        let conn = conn().await;
        mark_reported(&conn, &[]).await.unwrap();

        let run = begin(&conn, "derive").await.unwrap();
        mark_reported(&conn, &[run]).await.unwrap();
        mark_reported(&conn, &[run]).await.unwrap();
        assert!(reported(&conn, run).await.is_some());
        assert_eq!(closed(&conn, run).await, (RUNNING.to_string(), None));
    }
}
