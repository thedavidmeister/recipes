//! Turso/libSQL connection + a tiny versioned migration runner.
//!
//! Migrations are the versioned `.sql` files in `backend/migrations/`, embedded
//! at build time. The runner records applied versions in `_migrations` and
//! applies pending ones in ascending order. `libsql` speaks both a local file
//! and remote Turso, so the same code migrates dev and prod: point
//! `DATABASE_URL` at `file:recipes.db` for local, or the `libsql://…` URL (with
//! `TURSO_AUTH_TOKEN`) for Turso.
//!
//! **The scheme must be explicit and recognized.** Anything else is a hard error,
//! because the alternative is the failure this rule was written for: a
//! placeholder `DATABASE_URL` reached production, did not look like a remote URL,
//! and was silently treated as a *file path* — so the backend opened an ephemeral
//! SQLite inside its own container, served `/api/health` 200, and wrote every
//! recipe and session to a database that dies with the instance. Nothing looked
//! wrong. A deploy pointed at the wrong database must fail at startup, loudly,
//! not run beautifully against nothing.

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
/// Exactly two forms are accepted, and the scheme decides:
///
/// - `libsql://…` / `https://…` — remote Turso. Requires `TURSO_AUTH_TOKEN`.
/// - `file:…` — a local database. No token.
///
/// Anything else is an error, **including a bare path**. There is no default and
/// no fallback: see the module docs for what a silent fallback cost.
pub async fn open() -> anyhow::Result<Database> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL is required (`libsql://…` or `file:…`)"))?;
    let url = url.trim();

    if url.is_empty() {
        anyhow::bail!("DATABASE_URL is set but empty");
    }

    if url.starts_with("libsql://") || url.starts_with("https://") {
        let token = std::env::var("TURSO_AUTH_TOKEN")
            .map_err(|_| anyhow::anyhow!("TURSO_AUTH_TOKEN is required for a remote libsql URL"))?;
        if token.trim().is_empty() {
            anyhow::bail!("TURSO_AUTH_TOKEN is set but empty");
        }
        return Ok(Builder::new_remote(url.to_owned(), token).build().await?);
    }

    if let Some(path) = url.strip_prefix("file:") {
        if path.is_empty() {
            anyhow::bail!("DATABASE_URL is `file:` with no path");
        }
        return Ok(Builder::new_local(path).build().await?);
    }

    // The case that mattered: a placeholder, a typo, or a bare path. Previously
    // each of these opened a throwaway local file and looked healthy.
    anyhow::bail!(
        "DATABASE_URL has no recognized scheme: {url:?}. \
         Use `libsql://…` (Turso, needs TURSO_AUTH_TOKEN) or `file:…` (local). \
         A bare path is refused deliberately — it used to be silently accepted, \
         which let a placeholder run against a container-local database that \
         disappears on restart."
    )
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

    /// Every one of these used to open a throwaway local database and report
    /// success. A placeholder reached production that way: the service served
    /// `/api/health` 200 while writing to a file inside its own container, which
    /// dies with the instance. Pointing at the wrong database must be a startup
    /// failure, not a healthy-looking lie.
    ///
    /// Serialised onto one test because they share process-wide env.
    #[tokio::test]
    async fn database_url_fails_loud_rather_than_falling_back() {
        let restore = std::env::var("DATABASE_URL").ok();

        std::env::remove_var("DATABASE_URL");
        assert!(
            open().await.is_err(),
            "unset must not default to a local file"
        );

        for hostile in [
            "placeholder", // what actually happened
            "changeme",
            "recipes.db", // a bare path
            "/var/data/recipes.db",
            "postgres://x/y", // a real URL, wrong scheme
            "libsql:/typo",   // one slash short
            "   ",
        ] {
            std::env::set_var("DATABASE_URL", hostile);
            assert!(
                open().await.is_err(),
                "{hostile:?} must be refused, not opened as a local file"
            );
        }

        std::env::set_var("DATABASE_URL", "file:");
        assert!(
            open().await.is_err(),
            "`file:` with no path must be refused"
        );

        // A remote URL without a token must not fall through to anything local.
        let token = std::env::var("TURSO_AUTH_TOKEN").ok();
        std::env::remove_var("TURSO_AUTH_TOKEN");
        std::env::set_var("DATABASE_URL", "libsql://example.turso.io");
        assert!(
            open().await.is_err(),
            "remote without a token must be refused"
        );
        if let Some(t) = token {
            std::env::set_var("TURSO_AUTH_TOKEN", t);
        }

        // The one accepted local form still works.
        std::env::set_var("DATABASE_URL", "file::memory:");
        assert!(open().await.is_ok(), "`file:` is the way to ask for local");

        match restore {
            Some(v) => std::env::set_var("DATABASE_URL", v),
            None => std::env::remove_var("DATABASE_URL"),
        }
    }

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
            "login_completions",
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
