//! Turso/libSQL connection + a tiny versioned migration runner.
//!
//! Migrations are the versioned `.sql` files in `backend/migrations/`, embedded
//! at build time. The runner records applied versions in `_migrations` and
//! applies pending ones in ascending order. `libsql` speaks both a local file
//! and remote Turso, so the same code migrates dev and prod: point
//! `DATABASE_URL` at `file:recipes.db` for local, or the `libsql://…` URL (with
//! `TURSO_AUTH_TOKEN`) for Turso.

use libsql::{Builder, Connection, Database};

/// `(version, sql)` pairs, embedded at compile time, applied in ascending order.
/// Append new migrations here as `NNNN_*.sql` files with the next integer.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_raw_imports.sql")),
    (3, include_str!("../migrations/0003_auth.sql")),
];

/// Open the database described by `DATABASE_URL`.
///
/// A `libsql://` / `https://` URL opens a remote Turso database and requires
/// `TURSO_AUTH_TOKEN`; anything else (a `file:` URL or a bare path) opens a
/// local database with no token. Defaults to `file:recipes.db`.
pub async fn open() -> anyhow::Result<Database> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "file:recipes.db".to_string());

    if url.starts_with("libsql://") || url.starts_with("https://") {
        let token = std::env::var("TURSO_AUTH_TOKEN")
            .map_err(|_| anyhow::anyhow!("TURSO_AUTH_TOKEN is required for a remote libsql URL"))?;
        Ok(Builder::new_remote(url, token).build().await?)
    } else {
        let path = url.strip_prefix("file:").unwrap_or(&url);
        Ok(Builder::new_local(path).build().await?)
    }
}

/// Apply any migrations not yet recorded in `_migrations`. Idempotent.
pub async fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
    )
    .await?;

    let applied = highest_applied(conn).await?;
    for (version, sql) in MIGRATIONS {
        if *version <= applied {
            continue;
        }
        conn.execute_batch(sql).await?;
        conn.execute(
            "INSERT INTO _migrations (version) VALUES (?1)",
            libsql::params![*version],
        )
        .await?;
        tracing::info!("applied migration {version}");
    }
    Ok(())
}

async fn highest_applied(conn: &Connection) -> anyhow::Result<i64> {
    let mut rows = conn
        .query("SELECT COALESCE(MAX(version), 0) FROM _migrations", ())
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("expected a row from MAX(version)"))?;
    Ok(row.get::<i64>(0)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrate_creates_schema_and_is_idempotent() {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();

        migrate(&conn).await.unwrap();
        migrate(&conn).await.unwrap(); // second run must apply nothing

        // Both halves of the corpus exist and are queryable: `recipes` is the
        // derived view, `raw_imports` what the sources actually said. The auth
        // tables gate all of it (#25).
        for table in [
            "recipes",
            "raw_imports",
            "users",
            "login_attempts",
            "sessions",
        ] {
            let mut rows = conn
                .query(&format!("SELECT COUNT(*) FROM {table}"), ())
                .await
                .unwrap_or_else(|e| panic!("{table} must exist: {e}"));
            let count = rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap();
            assert_eq!(count, 0);
        }

        // Every migration is recorded — asserted against the list rather than a
        // literal, so adding one does not fail a test about idempotence.
        let latest = MIGRATIONS.iter().map(|(v, _)| *v).max().unwrap();
        assert_eq!(highest_applied(&conn).await.unwrap(), latest);
    }
}
