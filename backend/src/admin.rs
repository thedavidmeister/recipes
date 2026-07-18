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

    let db = &state.db;
    let recipes = scalar(db, "SELECT count(*) FROM recipes").await?;
    let raw = scalar(db, "SELECT count(*) FROM raw_imports").await?;
    let enriched = scalar(db, "SELECT count(*) FROM ingredient_structures").await?;
    let running = scalar(db, "SELECT count(*) FROM runs WHERE status = 'running'").await?;
    let enriched_pct = if recipes > 0 {
        (enriched as f64) * 100.0 / (recipes as f64)
    } else {
        0.0
    };

    Ok(Json(HealthStats {
        recipes,
        raw,
        enriched,
        enriched_pct,
        by_model: model_counts(db).await?,
        recent_runs: recent_runs(db).await?,
        running,
    }))
}

/// A one-row, one-column `i64` query — the `count(*)`s.
async fn scalar(conn: &Connection, sql: &str) -> Result<i64, AppError> {
    let mut rows = conn.query(sql, ()).await.map_err(query_err)?;
    let row = rows
        .next()
        .await
        .map_err(query_err)?
        .ok_or_else(|| AppError::Internal("health query returned no row".into()))?;
    row.get::<i64>(0).map_err(decode_err)
}

async fn model_counts(conn: &Connection) -> Result<Vec<ModelCount>, AppError> {
    let mut rows = conn
        .query(
            "SELECT model, count(*) FROM ingredient_structures
             GROUP BY model ORDER BY count(*) DESC",
            (),
        )
        .await
        .map_err(query_err)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(query_err)? {
        out.push(ModelCount {
            model: row.get::<String>(0).map_err(decode_err)?,
            count: row.get::<i64>(1).map_err(decode_err)?,
        });
    }
    Ok(out)
}

async fn recent_runs(conn: &Connection) -> Result<Vec<RunRow>, AppError> {
    let mut rows = conn
        .query(
            "SELECT id, kind, status, started_at, finished_at
             FROM runs ORDER BY id DESC LIMIT 20",
            (),
        )
        .await
        .map_err(query_err)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(query_err)? {
        out.push(RunRow {
            id: row.get::<i64>(0).map_err(decode_err)?,
            kind: row.get::<String>(1).map_err(decode_err)?,
            status: row.get::<String>(2).map_err(decode_err)?,
            started_at: row.get::<i64>(3).map_err(decode_err)?,
            finished_at: row.get::<Option<i64>>(4).map_err(decode_err)?,
        });
    }
    Ok(out)
}

fn query_err(e: libsql::Error) -> AppError {
    AppError::Internal(format!("health query failed: {e}"))
}

fn decode_err(e: libsql::Error) -> AppError {
    AppError::Internal(format!("health decode failed: {e}"))
}
