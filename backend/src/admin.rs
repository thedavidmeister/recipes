//! Admin-only operational views — currently the health dashboard.
//!
//! Session-gated like the rest of the app, then narrowed to the single configured
//! admin (`ADMIN_TELEGRAM_USER_ID`, see [`crate::auth::is_admin`]). The data here is
//! non-sensitive corpus/run aggregates the read-only token could already reach, but
//! the *view* is the operator's, so it is gated to them and computed server-side
//! rather than having the browser query internal tables (`runs`, `raw_imports`).

use axum::{extract::State, Extension, Json};
use libsql::Connection;
use serde::Serialize;

use crate::{auth::CurrentUser, error::AppError, AppState};

/// Corpus + enrichment + run health, as one snapshot.
#[derive(Debug, Serialize)]
pub struct HealthStats {
    /// Rows in `recipes` (the derived view the app reads).
    recipes: i64,
    /// Rows in `raw_imports` (source payloads).
    raw: i64,
    /// Rows in `ingredient_structures` (recipes with a structured reading).
    enriched: i64,
    /// `enriched` as a percentage of `recipes`; 0 when the corpus is empty.
    enriched_pct: f64,
    /// Enrichment counts by the model that produced them — provenance at a glance.
    by_model: Vec<ModelCount>,
    /// The most recent runs, newest first.
    recent_runs: Vec<RunRow>,
    /// Runs still `running` — a positive count long after `started_at` is the
    /// died-mid-flight signal the `runs` table exists to surface.
    running: i64,
}

#[derive(Debug, Serialize)]
struct ModelCount {
    model: String,
    count: i64,
}

#[derive(Debug, Serialize)]
struct RunRow {
    id: i64,
    kind: String,
    status: String,
    started_at: i64,
    finished_at: Option<i64>,
}

/// `GET /api/admin/health` — the dashboard's data. Session-gated by the router;
/// admin-gated here.
pub async fn health(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<HealthStats>, AppError> {
    if !crate::auth::is_admin(&state, &user.telegram_user_id) {
        return Err(AppError::Forbidden("admin only".into()));
    }

    // One retryable unit: all reads, so re-running on a transient Turso failure
    // (#130) can only re-read.
    let stats = state
        .with_db(move |db| async move {
            let recipes = scalar(&db, "SELECT count(*) FROM recipes").await?;
            let raw = scalar(&db, "SELECT count(*) FROM raw_imports").await?;
            let enriched = scalar(&db, "SELECT count(*) FROM ingredient_structures").await?;
            let running = scalar(&db, "SELECT count(*) FROM runs WHERE status = 'running'").await?;
            let enriched_pct = if recipes > 0 {
                (enriched as f64) * 100.0 / (recipes as f64)
            } else {
                0.0
            };
            Ok(HealthStats {
                recipes,
                raw,
                enriched,
                enriched_pct,
                by_model: model_counts(&db).await?,
                recent_runs: recent_runs(&db).await?,
                running,
            })
        })
        .await
        .map_err(|e| AppError::Internal(format!("health query failed: {e:#}")))?;

    Ok(Json(stats))
}

/// A one-row, one-column `i64` query — the `count(*)`s.
async fn scalar(conn: &Connection, sql: &str) -> anyhow::Result<i64> {
    let mut rows = conn.query(sql, ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("health query returned no row"))?;
    Ok(row.get::<i64>(0)?)
}

async fn model_counts(conn: &Connection) -> anyhow::Result<Vec<ModelCount>> {
    let mut rows = conn
        .query(
            "SELECT model, count(*) FROM ingredient_structures
             GROUP BY model ORDER BY count(*) DESC",
            (),
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(ModelCount {
            model: row.get::<String>(0)?,
            count: row.get::<i64>(1)?,
        });
    }
    Ok(out)
}

async fn recent_runs(conn: &Connection) -> anyhow::Result<Vec<RunRow>> {
    let mut rows = conn
        .query(
            "SELECT id, kind, status, started_at, finished_at
             FROM runs ORDER BY id DESC LIMIT 20",
            (),
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(RunRow {
            id: row.get::<i64>(0)?,
            kind: row.get::<String>(1)?,
            status: row.get::<String>(2)?,
            started_at: row.get::<i64>(3)?,
            finished_at: row.get::<Option<i64>>(4)?,
        });
    }
    Ok(out)
}
