//! recipes backend — ingest, the corpus store, and `derive`.
//!
//! Deploys to Render. It does what a browser cannot: fetch external pages/APIs
//! server-side (past CORS and bot walls), hold the Turso *write* token, and own
//! what enters the corpus. Normalization runs here, natively — the client drives
//! ingestion and the server performs it.
//!
//! `derive` rebuilds the `recipes` view from stored payloads. It is an offline
//! command over data we already hold, not a request path — no page is fetched
//! and no client is involved.
//!
//! Usage:
//!   recipe-backend                    serve
//!   recipe-backend derive [<source>]  rebuild `recipes` from `raw_imports`

mod db;
mod derive;
mod error;
mod ingest;
mod proxy;
mod recipes;

use axum::{
    routing::{get, post},
    Json, Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

/// Shared handler state: the SSRF-guarded HTTP client (proxy) and a Turso/libSQL
/// connection (write-gateway).
#[derive(Clone)]
pub struct AppState {
    pub http: reqwest::Client,
    pub db: libsql::Connection,
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "recipe_backend=debug,tower_http=info,info".into()),
        )
        .init();

    // `recipe-backend migrate` applies pending DB migrations, then exits.
    if std::env::args().nth(1).as_deref() == Some("migrate") {
        let database = db::open().await?;
        let conn = database.connect()?;
        db::migrate(&conn).await?;
        tracing::info!("migrations up to date");
        return Ok(());
    }

    // `recipe-backend derive [<source>]` rebuilds the `recipes` view from the
    // payloads in `raw_imports`, then exits. No network: it only reads what we
    // already hold, which is the point — re-fetching is not a reliable recovery
    // plan (sources 502 scrapers, die, and paywall).
    if std::env::args().nth(1).as_deref() == Some("derive") {
        let database = db::open().await?;
        let conn = database.connect()?;
        db::migrate(&conn).await?;
        let source = std::env::args().nth(2);
        let report = derive::derive(&conn, source.as_deref()).await?;
        tracing::info!(
            read = report.read,
            derived = report.derived,
            skipped = report.skipped,
            "derive complete"
        );
        return Ok(());
    }

    // Open the DB, ensure the schema is current, and build the shared state.
    let database = db::open().await?;
    let conn = database.connect()?;
    db::migrate(&conn).await?;
    let state = AppState {
        http: proxy::build_client()?,
        db: conn,
    };

    let api = Router::new()
        .route("/health", get(health))
        .route("/ingest", post(ingest::ingest))
        .with_state(state);

    let app = Router::new()
        .nest("/api", api)
        // Permissive for now. Before deploy, restrict this to the frontend
        // origin so the proxy can't be used as an open fetch relay.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("recipes backend listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
