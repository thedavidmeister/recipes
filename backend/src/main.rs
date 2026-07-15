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

mod auth;
mod db;
mod derive;
mod error;
mod ingest;
mod proxy;
mod recipes;

use axum::{
    http::Method,
    routing::{get, post},
    Json, Router,
};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

/// Shared handler state: the SSRF-guarded HTTP client, a Turso/libSQL
/// connection, and the Telegram config auth runs on.
#[derive(Clone)]
pub struct AppState {
    pub http: reqwest::Client,
    pub db: libsql::Connection,
    pub telegram: auth::TelegramConfig,
    pub cookie: auth::CookieConfig,
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// Build the HTTP surface.
///
/// Split out of `main` so the auth gate can be tested against the **real**
/// router rather than a hand-assembled lookalike: "every endpoint requires a
/// session" is a claim about this wiring, so a test that rebuilt the wiring
/// would prove nothing about what actually serves traffic.
pub fn app(state: AppState) -> Router {
    // Everything the corpus touches. Auth is mandatory (#25): since #29 the
    // client drives ingestion and the server performs it, so `/ingest` is what a
    // search does — gating it gates search, deliberately.
    let guarded = Router::new()
        .route("/ingest", post(ingest::ingest))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_session,
        ));

    // The only endpoints reachable without a session, each because requiring one
    // would be circular or wrong:
    //   /health           — a liveness probe the host calls, holding no session.
    //   /auth/start|poll  — how a session is obtained in the first place.
    //   /telegram/webhook — called by Telegram, not a browser; it carries no
    //                       session and authenticates by its own secret instead.
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/start", post(auth::start))
        .route("/auth/poll", post(auth::poll))
        .route("/telegram/webhook", post(auth::webhook));

    let api = Router::new().merge(guarded).merge(public).with_state(state);

    Router::new()
        .nest("/api", api)
        .layer(cors())
        .layer(TraceLayer::new_for_http())
}

/// CORS for a credentialed, cross-origin, same-site frontend.
///
/// **This is not a security control.** CORS is browser-enforced — `curl` ignores
/// it entirely, and the session check is what actually guards these endpoints. A
/// previous revision described restricting CORS as if it stopped abuse; it does
/// not, and cannot.
///
/// It has to be explicit anyway, for a browser reason rather than a security
/// one: a credentialed request may not be answered with
/// `Access-Control-Allow-Origin: *`, so the permissive layer would silently stop
/// the browser sending the session cookie at all. `CORS_ALLOWED_ORIGIN` names the
/// frontend; unset means dev, where any origin may ask *and still needs a valid
/// session to get anything*.
///
/// Methods are enumerated rather than `Any` for the same reason the origin is:
/// `Any` is **illegal** with credentials. tower-http panics on
/// `Allow-Credentials: true` + `Allow-Methods: *`, so getting this wrong is a
/// startup crash rather than a bad response.
fn cors() -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_credentials(true);

    match std::env::var("CORS_ALLOWED_ORIGIN") {
        Ok(origin) if !origin.trim().is_empty() => {
            let origins: Vec<_> = origin
                .split(',')
                .filter_map(|o| o.trim().parse::<axum::http::HeaderValue>().ok())
                .collect();
            base.allow_origin(origins)
        }
        // `mirror_request` echoes the caller's origin — `*` with credentials made
        // legal. Fine for dev, and not a session leak even there: the cookie is
        // `SameSite=Lax`, so a cross-site page's request never carries it however
        // permissive CORS is. Prod names its origin anyway.
        _ => base.allow_origin(AllowOrigin::mirror_request()),
    }
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
    // Auth is mandatory, so missing Telegram config is a startup error: a
    // backend that cannot mint a login can serve nothing, and failing here beats
    // discovering it on the first request.
    let state = AppState {
        http: proxy::build_client()?,
        db: conn.clone(),
        telegram: auth::TelegramConfig::from_env()?,
        cookie: auth::CookieConfig::from_env(),
    };

    // Expired rows are already refused on read, so this only reclaims space.
    if let Err(e) = auth::sweep_expired(&conn).await {
        tracing::warn!("could not sweep expired auth rows: {e}");
    }

    let app = app(state);

    let addr: String = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("recipes backend listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Does the front door actually lock?
///
/// Every other auth test checks a *piece* — that a nonce hashes, that a claim is
/// single-use. None of them prove the claim that matters, which is a property of
/// the router wiring: **auth is mandatory**. So these drive the real [`app`],
/// because a lookalike router assembled in a test would prove only that the
/// lookalike locks.
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn test_app() -> (Router, libsql::Connection) {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        db::migrate(&conn).await.unwrap();
        let state = AppState {
            http: proxy::build_client().unwrap(),
            db: conn.clone(),
            telegram: auth::TelegramConfig {
                bot_token: "test-token".into(),
                bot_username: "testbot".into(),
                webhook_secret: "test-webhook-secret".into(),
            },
            cookie: auth::CookieConfig {
                domain: None,
                secure: false,
            },
        };
        (app(state), conn)
    }

    fn ingest_req(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/api/ingest")
            .header("content-type", "application/json");
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::from(
            r#"{"url":"https://www.themealdb.com/api/json/v1/1/lookup.php?i=1"}"#,
        ))
        .unwrap()
    }

    /// The headline: an anonymous caller cannot reach the corpus. Since #29
    /// `/ingest` is what a search does, so this is also "you cannot search
    /// without logging in" — deliberate, per the ruling on #25.
    #[tokio::test]
    async fn ingest_is_closed_without_a_session() {
        let (app, _conn) = test_app().await;
        let res = app.oneshot(ingest_req(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// A guessed or malformed cookie is not a session. Note this would pass even
    /// if the gate were absent *and* ingest happened to fail — so it is paired
    /// with `a_valid_session_passes_the_gate`, which pins that the gate is what
    /// answers here.
    #[tokio::test]
    async fn a_bogus_cookie_is_not_a_session() {
        let (app, _conn) = test_app().await;
        for header in [
            "recipes_session=deadbeef",
            "recipes_session=",
            "other=abc",
            "garbage",
            // A name must match whole: a prefix must not satisfy the gate.
            "xrecipes_session=deadbeef",
        ] {
            let res = app.clone().oneshot(ingest_req(Some(header))).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNAUTHORIZED,
                "{header:?} must not authenticate"
            );
        }
    }

    /// The other half of the proof: with a real session the request gets *past*
    /// the gate. It then fails on the network (no upstream in a test), which is
    /// exactly the point — a 401 here would mean the gate rejects valid
    /// sessions, and only distinguishing the two shows the gate is doing the
    /// work rather than something else failing first.
    #[tokio::test]
    async fn a_valid_session_passes_the_gate() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(ingest_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_ne!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "a live session must get past the gate"
        );
    }

    /// An expired session is dead on read, not merely swept later.
    #[tokio::test]
    async fn an_expired_session_is_refused() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        conn.execute("UPDATE sessions SET expires_at = unixepoch() - 1", ())
            .await
            .unwrap();
        let res = app
            .oneshot(ingest_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// The session must be found among other cookies — real browsers send more
    /// than one.
    #[tokio::test]
    async fn the_session_is_found_alongside_other_cookies() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(ingest_req(Some(&format!(
                "theme=dark; recipes_session={token}; lang=en"
            ))))
            .await
            .unwrap();
        assert_ne!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// Health has to answer an unauthenticated prober or the host cannot tell if
    /// we are alive.
    #[tokio::test]
    async fn health_is_reachable_without_a_session() {
        let (app, _conn) = test_app().await;
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// Login cannot require a login.
    #[tokio::test]
    async fn auth_start_is_reachable_without_a_session() {
        let (app, _conn) = test_app().await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/start")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// The webhook is public, so its own secret is the only thing standing
    /// between a stranger and a forged login for any Telegram id.
    #[tokio::test]
    async fn the_webhook_rejects_a_forged_origin() {
        let (app, _conn) = test_app().await;
        let forged = r#"{"message":{"text":"/start abc","from":{"id":1,"username":"mallory"}}}"#;

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/telegram/webhook")
                    .header("content-type", "application/json")
                    .body(Body::from(forged))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "no secret token must be refused"
        );

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/telegram/webhook")
                    .header("content-type", "application/json")
                    .header("x-telegram-bot-api-secret-token", "wrong")
                    .body(Body::from(forged))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "wrong secret");
    }
}
