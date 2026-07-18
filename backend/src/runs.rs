//! Run generation: a monotonic id per pipeline invocation (#11 write-path
//! hardening).
//!
//! Every invocation — the `/api/ingest` pipeline, or a CLI `enrich`/`derive` —
//! opens a run here at start and closes it at end. The DB-assigned `id` is a
//! total order free of the clock skew between Render and a CLI box, so corpus
//! writes stamp it and guard on it (only an equal-or-newer run overwrites) and no
//! concurrent or partial run clobbers another. The row is also the audit trail: a
//! run still `running` long after `started_at` is one that died mid-flight.

use libsql::Connection;

/// A run's terminal state, for [`finish`].
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";

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

/// Close a run: stamp `finished_at` and the final `status` ([`COMPLETED`] /
/// [`FAILED`]). Leaving a run un-closed is what marks it as died-mid-flight.
pub async fn finish(conn: &Connection, run_id: i64, status: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE runs SET finished_at = unixepoch(), status = ?2 WHERE id = ?1",
        libsql::params![run_id, status],
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

        finish(&conn, first, COMPLETED).await.unwrap();
        let mut rows = conn
            .query(
                "SELECT status, finished_at FROM runs WHERE id = ?1",
                libsql::params![first],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "completed");
        assert!(row.get::<Option<i64>>(1).unwrap().is_some());
    }
}
