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

/// Version numbers deliberately left empty, and why each one is.
///
/// The ledger below is checked for holes (`migration_ledger_is_well_formed`), because
/// a hole is nearly always a migration someone forgot to register — a `.sql` file that
/// silently never runs. A hole that is *meant* has to be written down here, so the
/// check stays sharp instead of being switched off.
///
/// A number in this list is **burnt, not free**: see the comment on 23 below. Filling
/// one back in is the exact failure the runner cannot survive.
///
/// `cfg(test)` because only the check reads it — [`migrate`] iterates [`MIGRATIONS`]
/// and has no use for a number with no SQL behind it. It still lives here rather than
/// down in the test module, because it belongs to the ledger: a reservation that is not
/// visible beside the list it applies to is a reservation nobody will honour.
#[cfg(test)]
const RESERVED: &[i64] = &[
    // Claimed by work in flight on another branch (#96) while 21–23 were being
    // written, then never used. It is below the production floor now, so it can
    // never be filled.
    20,
];

/// `(version, sql)` pairs, embedded at compile time, applied in ascending order.
/// Append new migrations here as `NNNN_*.sql` files with the next integer.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_raw_imports.sql")),
    (3, include_str!("../migrations/0003_auth.sql")),
    (
        4,
        include_str!("../migrations/0004_ingredient_structures.sql"),
    ),
    (5, include_str!("../migrations/0005_runs.sql")),
    (6, include_str!("../migrations/0006_sessions.sql")),
    (
        7,
        include_str!("../migrations/0007_rename_pick_sessions.sql"),
    ),
    (8, include_str!("../migrations/0008_step_structures.sql")),
    (9, include_str!("../migrations/0009_kitchens.sql")),
    (10, include_str!("../migrations/0010_primary_kitchen.sql")),
    (11, include_str!("../migrations/0011_pick_lobby.sql")),
    (12, include_str!("../migrations/0012_no_guests.sql")),
    (13, include_str!("../migrations/0013_expiring_invites.sql")),
    (
        14,
        include_str!("../migrations/0014_equipment_structures.sql"),
    ),
    (
        15,
        include_str!("../migrations/0015_recipe_total_seconds.sql"),
    ),
    (16, include_str!("../migrations/0016_meal_type.sql")),
    (17, include_str!("../migrations/0017_pick_time_cap.sql")),
    (18, include_str!("../migrations/0018_buy_checks.sql")),
    (19, include_str!("../migrations/0019_time_cap_default.sql")),
    // 20 is reserved for work in flight on another branch (#96). The runner below
    // applies by `MAX(version)`, so reserving a number is only half the job: one that
    // lands *after* a higher number has already run on production never applies at
    // all. Whichever deploys first sets the floor, so anything behind it must be
    // renumbered above it before it merges.
    (21, include_str!("../migrations/0021_pantry_pretick.sql")),
    (
        22,
        include_str!("../migrations/0022_recipe_fully_timed.sql"),
    ),
    // 23, not the free 20 above it: 21 and 22 have already run on production, so
    // anything numbered 20 can never apply again — filling a hole below the floor is
    // the exact failure the comment above warns about. The next number above
    // *everything*, including what is unmerged on other branches, is the only choice
    // that is safe whichever deploys first.
    (
        23,
        include_str!("../migrations/0023_nutrition_structures.sql"),
    ),
    // 24 for the same reason 23 was not 20: production `_migrations` is at 23, so the
    // next number above everything — including anything unmerged elsewhere — is the only
    // one guaranteed to apply whichever branch deploys first.
    (
        24,
        include_str!("../migrations/0024_meal_time_structures.sql"),
    ),
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
    // Env is read here and nowhere else: `resolve` is pure, so the rules can be
    // tested without mutating process-global state. `std::env::set_var` is
    // unsound under a threaded test runner — it is `unsafe` in edition 2024 —
    // and two tests racing on the same variable is the kind of failure that
    // shows up as an unrelated segfault.
    match resolve(
        std::env::var("DATABASE_URL").ok().as_deref(),
        std::env::var("TURSO_AUTH_TOKEN").ok().as_deref(),
    )? {
        Target::Remote { url, token } => Ok(Builder::new_remote(url, token).build().await?),
        Target::Local { path } => Ok(Builder::new_local(path).build().await?),
    }
}

/// Where [`open`] was told to connect.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Remote { url: String, token: String },
    Local { path: String },
}

/// Decide what `DATABASE_URL` means, or refuse.
///
/// Pure: takes the values rather than reading them, so the rules below are
/// testable without touching the environment.
fn resolve(url: Option<&str>, token: Option<&str>) -> anyhow::Result<Target> {
    let url = url
        .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required (`libsql://…` or `file:…`)"))?
        .trim();

    if url.is_empty() {
        anyhow::bail!("DATABASE_URL is set but empty");
    }

    if url.starts_with("libsql://") || url.starts_with("https://") {
        let token = token.ok_or_else(|| {
            anyhow::anyhow!("TURSO_AUTH_TOKEN is required for a remote libsql URL")
        })?;
        if token.trim().is_empty() {
            anyhow::bail!("TURSO_AUTH_TOKEN is set but empty");
        }
        return Ok(Target::Remote {
            url: url.to_owned(),
            token: token.to_owned(),
        });
    }

    if let Some(path) = url.strip_prefix("file:") {
        if path.is_empty() {
            anyhow::bail!("DATABASE_URL is `file:` with no path");
        }
        return Ok(Target::Local {
            path: path.to_owned(),
        });
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
    #[test]
    fn database_url_fails_loud_rather_than_falling_back() {
        assert!(
            resolve(None, None).is_err(),
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
            "",
            "file:", // no path
        ] {
            assert!(
                resolve(Some(hostile), Some("tok")).is_err(),
                "{hostile:?} must be refused, not opened as a local file"
            );
        }

        // A remote URL must not fall through to anything local when the token is
        // missing or blank.
        assert!(resolve(Some("libsql://x.turso.io"), None).is_err());
        assert!(resolve(Some("libsql://x.turso.io"), Some("  ")).is_err());
    }

    /// The two accepted forms, and that the scheme picks the right one.
    #[test]
    fn recognized_schemes_resolve() {
        assert_eq!(
            resolve(Some("libsql://x.turso.io"), Some("tok")).unwrap(),
            Target::Remote {
                url: "libsql://x.turso.io".into(),
                token: "tok".into()
            }
        );
        assert_eq!(
            resolve(Some("https://x.turso.io"), Some("tok")).unwrap(),
            Target::Remote {
                url: "https://x.turso.io".into(),
                token: "tok".into()
            }
        );
        assert_eq!(
            resolve(Some("file:recipes.db"), None).unwrap(),
            Target::Local {
                path: "recipes.db".into()
            }
        );
        // Local needs no token — asking for one would be theatre.
        assert!(resolve(Some("file::memory:"), None).is_ok());
    }

    /// **The migration ledger is a gate, not a convention (#176/5).**
    ///
    /// [`migrate`] applies by `MAX(version)`, so the ledger's *shape* decides whether
    /// a migration ever runs at all, and every way of getting that shape wrong is
    /// silent: the build is green, the tests pass, and the column simply is not there
    /// in production. Hand-resolution on every stacked branch for a week is the
    /// evidence that a comment was not enough. Four invariants, each for a real way
    /// this has gone wrong:
    ///
    /// 1. **Strictly ascending.** A number out of order below the floor never applies.
    /// 2. **The registered SQL is the file the number names.** Appending an entry by
    ///    copying the line above it and editing only the version leaves two versions
    ///    running one file — and the new migration never runs.
    /// 3. **Every file is registered.** Writing `00NN_thing.sql` and not adding it to
    ///    [`MIGRATIONS`] is a migration that does not exist; nothing else notices.
    /// 4. **No undocumented hole.** A gap is nearly always (3). One that is deliberate
    ///    goes in [`RESERVED`] with its reason, so the check stays on.
    ///
    /// What this *cannot* check is the one that needs production: that every new
    /// version exceeds the highest already applied there. `_migrations` is the only
    /// place that answer lives, so it belongs to the audit skill, which has the
    /// read-only token.
    #[test]
    fn migration_ledger_is_well_formed() {
        use std::collections::BTreeMap;

        // 1. Strictly ascending.
        for pair in MIGRATIONS.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "migrations are applied in list order by MAX(version): {} then {} is \
                 out of order, and the lower one would never apply",
                pair[0].0,
                pair[1].0
            );
        }

        // The `NNNN_name.sql` files on disk, keyed by their numeric prefix.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut on_disk: BTreeMap<i64, (String, String)> = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).expect("backend/migrations must exist") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                continue;
            }
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let version: i64 = name
                .split('_')
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(|| panic!("{name} must be named NNNN_description.sql"));
            let sql = std::fs::read_to_string(&path).unwrap();
            if let Some((other, _)) = on_disk.insert(version, (name.clone(), sql)) {
                panic!("{other} and {name} both claim version {version}");
            }
        }

        // 2. Each entry embeds the file its own number names — not the one above it.
        for (version, sql) in MIGRATIONS {
            let (name, on_disk_sql) = on_disk.get(version).unwrap_or_else(|| {
                panic!("migration {version} is registered but has no NNNN_*.sql file")
            });
            assert_eq!(
                sql, on_disk_sql,
                "migration {version} is registered with SQL that is not {name} — an \
                 entry copied from the line above it runs that file twice and this \
                 one never"
            );
        }

        // 3. Every file on disk is registered. An unregistered one never runs.
        let registered: Vec<i64> = MIGRATIONS.iter().map(|(v, _)| *v).collect();
        for (version, (name, _)) in &on_disk {
            assert!(
                registered.contains(version),
                "{name} is not in MIGRATIONS, so it never runs — add it, do not \
                 rely on the file existing"
            );
        }

        // 4. No hole that nobody wrote down.
        let max = *registered.last().expect("MIGRATIONS is not empty");
        for version in 1..=max {
            assert!(
                registered.contains(&version) || RESERVED.contains(&version),
                "migration {version} is missing from MIGRATIONS and is not in \
                 RESERVED — either register the file that was meant to be there, or \
                 record why the number is empty"
            );
        }
        for version in RESERVED {
            assert!(
                !registered.contains(version),
                "migration {version} is registered *and* listed as RESERVED — the \
                 reservation is stale and now hides a real hole"
            );
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
            "ingredient_structures",
            "nutrition_structures",
            "meal_time_structures",
            "runs",
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

    /// One plan's cap, read back.
    async fn cap_of(conn: &Connection, channel: &str) -> Option<i64> {
        let mut rows = conn
            .query(
                "SELECT max_total_seconds FROM pick_sessions WHERE channel_id = ?1",
                libsql::params![channel],
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .expect("the plan is there")
            .get::<Option<i64>>(0)
            .unwrap()
    }

    /// The schema states what a plan is born as (#163): an insert that names no cap
    /// gets half an hour, the same number the create handler applies, so a row
    /// written around the handler cannot mean something different from one written
    /// through it.
    ///
    /// And the default is a *starting point*, not a floor — an insert that names
    /// `NULL` still gets "Any". A DEFAULT only fills a column the INSERT left out,
    /// which is exactly what lets the lobby lift a cap and what leaves plans made
    /// before #163 alone.
    #[tokio::test]
    async fn the_time_cap_column_defaults_to_thirty_minutes() {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        migrate(&conn).await.unwrap();

        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by) VALUES ('unstated', 'alice')",
            (),
        )
        .await
        .unwrap();
        assert_eq!(cap_of(&conn, "unstated").await, Some(1800));

        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by, max_total_seconds)
             VALUES ('any', 'alice', NULL)",
            (),
        )
        .await
        .unwrap();
        assert_eq!(cap_of(&conn, "any").await, None);
    }

    /// #163's default is what a plan is *born* as, never a rule applied backwards.
    ///
    /// The migration rebuilds `pick_sessions` (SQLite cannot ALTER a default), and a
    /// rebuild is exactly where a backfill would slip in unnoticed — a copy that
    /// omitted the column would hand every uncapped plan a 30-minute bound, and a
    /// started plan's bound is frozen for the roster swiping within it. So the SQL
    /// that ships is re-run over seeded rows here: every cap must come out the way
    /// it went in, including the NULLs. Re-running it is also how a migration that
    /// died mid-flight is retried, so this pins that too.
    #[tokio::test]
    async fn the_time_cap_rebuild_backfills_nothing_and_survives_a_rerun() {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        migrate(&conn).await.unwrap();

        for (channel, cap, started) in [
            ("open-any", None, None),
            ("open-capped", Some(7200i64), None),
            ("started-any", None, Some(1_700_000_000i64)),
        ] {
            conn.execute(
                "INSERT INTO pick_sessions
                    (channel_id, created_by, max_total_seconds, started_at)
                 VALUES (?1, 'alice', ?2, ?3)",
                libsql::params![channel, cap, started],
            )
            .await
            .unwrap();
        }

        let rebuild = MIGRATIONS
            .iter()
            .find(|(v, _)| *v == 19)
            .expect("migration 19 is the time-cap default")
            .1;
        conn.execute_batch(rebuild).await.unwrap();

        assert_eq!(cap_of(&conn, "open-any").await, None);
        assert_eq!(cap_of(&conn, "open-capped").await, Some(7200));
        assert_eq!(cap_of(&conn, "started-any").await, None);

        // Nothing was duplicated or dropped on the way through.
        let mut rows = conn
            .query("SELECT COUNT(*) FROM pick_sessions", ())
            .await
            .unwrap();
        let count = rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap();
        assert_eq!(count, 3);

        // …and the rebuilt table still carries the default, so a retry does not
        // quietly leave the schema one migration short of what it claims.
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by) VALUES ('after', 'alice')",
            (),
        )
        .await
        .unwrap();
        assert_eq!(cap_of(&conn, "after").await, Some(1800));
    }
}
