//! recipes backend — an SSRF-guarded fetch proxy.
//!
//! Deploys to Render. Its only job (for now) is to fetch external pages/APIs
//! server-side so the browser can get past CORS and bot walls; the Turso
//! write-gateway lands in a follow-up. Parsing happens client-side in
//! recipe-core WASM, not here.

mod db;
mod error;
mod proxy;

use axum::{
    routing::{get, post},
    Json, Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::proxy::AppState;

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

    let state = AppState::new()?;

    let api = Router::new()
        .route("/health", get(health))
        .route("/fetch", post(proxy::fetch))
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
