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
mod sync;
mod walk;

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
/// connection, the Telegram config auth runs on, and the infra key that guards
/// the ingest sync.
#[derive(Clone)]
pub struct AppState {
    pub http: reqwest::Client,
    pub db: libsql::Connection,
    pub telegram: auth::TelegramConfig,
    pub cookie: auth::CookieConfig,
    /// Authenticates the machine that triggers `/api/ingest` (#49) — a schedule,
    /// not a person. Never a user, and never a session.
    ///
    /// `None` disables ingest rather than the app: the rest of the service does
    /// not need this key, so a deployment missing it still serves. It is an
    /// `Option` so the unset case cannot be compared against — see
    /// [`auth::ingest_key_from_env`].
    pub ingest_key: Option<String>,
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
    // Machine-only. `/ingest` is a server-driven corpus sync (#49): a schedule
    // triggers it, not a person, so it authenticates with an `Authorization:
    // Bearer` key instead of a session. The frontend has no access to ingestion
    // at all — a valid session cookie does *not* open this door, which is the
    // point: the client no longer decides what enters the corpus.
    let machine = Router::new()
        .route("/ingest", post(ingest::ingest))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ));

    // Everything a person touches. Auth is mandatory (#25): the corpus is for a
    // known group, and #20 needs a headcount.
    let guarded = Router::new()
        // `/me` is guarded like everything else, which is what makes it useful:
        // the session cookie is HttpOnly, so the SPA cannot see whether it is
        // logged in. A 401 here *is* the answer.
        .route("/me", get(auth::me))
        // The `pick` engine (#47): a variety-first wander over the corpus. A
        // person-facing read, so it is session-gated like the rest.
        .route("/walk", get(walk::walk))
        .route("/auth/logout", post(auth::logout))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_session,
        ));

    // The only endpoints reachable without a session, each because requiring one
    // would be circular or wrong:
    //   /health           — a liveness probe the host calls, holding no session.
    //   /auth/complete    — redeems the bot's link; the secret in it IS the
    //                       authentication, and requiring a session to get one
    //                       would be circular.
    //   /telegram/webhook — called by Telegram, not a browser; it carries no
    //                       session and authenticates by its own secret instead.
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/complete", post(auth::complete))
        .route("/telegram/webhook", post(auth::webhook));

    let api = Router::new()
        .merge(machine)
        .merge(guarded)
        .merge(public)
        .with_state(state);

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
    //
    // The ingest key is the exception — it gates one scheduled endpoint, so
    // missing it costs a sync, not the service. Warn and serve; ingest itself
    // refuses while it is unset.
    let ingest_key = auth::ingest_key_from_env();
    if ingest_key.is_none() {
        tracing::warn!("INGEST_API_KEY is not set — /api/ingest is disabled; the corpus will go stale until it is configured");
    }
    let state = AppState {
        http: proxy::build_client()?,
        db: conn.clone(),
        telegram: auth::TelegramConfig::from_env()?,
        cookie: auth::CookieConfig::from_env()?,
        ingest_key,
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
        test_app_with_ingest_key(Some("test-ingest-key".into())).await
    }

    /// The router with ingest configured, or not. `None` is a real deployment
    /// state rather than a hypothetical — the key is optional (a backend without
    /// one still serves everything else), so the tests reach that state the same
    /// way the process does.
    async fn test_app_with_ingest_key(ingest_key: Option<String>) -> (Router, libsql::Connection) {
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
                webhook_secret: "test-webhook-secret".into(),
                frontend_base_url: "https://recipes.test".into(),
            },
            cookie: auth::CookieConfig {
                domain: None,
                secure: false,
            },
            ingest_key,
        };
        (app(state), conn)
    }

    /// A `GET /api/me` — the session-gated route the gate tests probe. `/me` is
    /// deliberate: it is cheap and deterministic, whereas `/api/ingest` is no
    /// longer session-gated at all (it is machine-only now, #49).
    fn me_req(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri("/api/me");
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::empty()).unwrap()
    }

    /// A `POST /api/ingest`. It takes no body — it triggers a server-driven sync
    /// (#49). An unauthenticated caller is rejected at the middleware and never
    /// reaches the sync, so these perform no network.
    fn ingest_req(auth: Option<&str>, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("POST").uri("/api/ingest");
        if let Some(v) = auth {
            b = b.header("authorization", v);
        }
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::empty()).unwrap()
    }

    /// The headline: an anonymous caller cannot reach the corpus (#25).
    #[tokio::test]
    async fn a_request_without_a_session_is_refused() {
        let (app, _conn) = test_app().await;
        let res = app.oneshot(me_req(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// A guessed or malformed cookie is not a session. Paired with
    /// `a_valid_session_passes_the_gate`, which pins that the gate is what answers
    /// here rather than the route simply being broken.
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
            let res = app.clone().oneshot(me_req(Some(header))).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNAUTHORIZED,
                "{header:?} must not authenticate"
            );
        }
    }

    /// Ingestion is machine-only (#49): no key, no entry. A missing gate would let
    /// this reach the handler and run a real sync (200, and real HTTP), so a 401
    /// here is what proves the middleware is actually wired.
    #[tokio::test]
    async fn ingest_requires_an_api_key() {
        let (app, _conn) = test_app().await;
        let res = app.oneshot(ingest_req(None, None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// A missing `INGEST_API_KEY` must cost a *sync*, not the service. The key
    /// guards one scheduled endpoint, so exiting over it would turn a stale
    /// corpus into an outage: no login, no reads, and no `/health` for the
    /// prober that would report it.
    #[tokio::test]
    async fn a_missing_ingest_key_does_not_take_the_app_down() {
        let (app, _conn) = test_app_with_ingest_key(None).await;
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

    /// The other half of that trade, and the one worth pinning: an unconfigured
    /// ingest is **closed**, never open.
    ///
    /// `Bearer ` is the case with teeth. Were the key a `String` defaulting to
    /// empty, an unset variable would compare equal to a bearer with nothing
    /// after the scheme — so forgetting the config would *unlock* ingestion to
    /// anyone. It answers 503 rather than 401 because no credential exists to be
    /// wrong about: the fault is the deployment's, and an operator reading 401
    /// would go hunting for a bad key instead.
    #[tokio::test]
    async fn without_a_key_configured_ingest_is_closed_not_open() {
        let (app, _conn) = test_app_with_ingest_key(None).await;
        for header in [
            None,
            Some("Bearer "),
            Some("Bearer"),
            Some(""),
            // Nor does the key some *other* deployment holds open this one.
            Some("Bearer test-ingest-key"),
        ] {
            let res = app.clone().oneshot(ingest_req(header, None)).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{header:?} must not reach an unconfigured ingest"
            );
        }
    }

    /// A wrong key — or the right key under the wrong scheme, or none at all — is
    /// not the key.
    #[tokio::test]
    async fn ingest_rejects_a_bad_api_key() {
        let (app, _conn) = test_app().await;
        for header in [
            "Bearer wrong",
            "Bearer ",
            // A prefix or an extension of the key must not satisfy it.
            "Bearer test-ingest-ke",
            "Bearer test-ingest-keys",
            // The scheme is not optional, and Basic is not Bearer.
            "test-ingest-key",
            "Basic test-ingest-key",
        ] {
            let res = app
                .clone()
                .oneshot(ingest_req(Some(header), None))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNAUTHORIZED,
                "{header:?} must not authenticate"
            );
        }
    }

    /// The property that makes "the frontend has no access to ingestion" true: a
    /// perfectly good browser session does not open this door. Only the key does,
    /// and the browser never holds it.
    #[tokio::test]
    async fn a_session_cookie_does_not_reach_ingestion() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(ingest_req(None, Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "a session must not authenticate a machine-only endpoint"
        );
    }

    /// The other half of the proof: with a real session the request gets *past*
    /// the gate and lands on a handler. We check `/api/me` — a lightweight authed
    /// route — rather than `/api/ingest`, which now triggers a real network sync.
    /// Asserting an exact 200 (rather than merely "not 401") is what makes this
    /// prove the middleware ran and passed: a bare `!= 401` would also be
    /// satisfied by a 500, or by the gate being absent entirely.
    #[tokio::test]
    async fn a_valid_session_passes_the_gate() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(me_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "a live session must pass the gate and reach the handler"
        );
    }

    fn walk_req(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri("/api/walk?len=5");
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::empty()).unwrap()
    }

    /// The walk is a person-facing read, so it is session-gated like the rest (#25).
    #[tokio::test]
    async fn walk_requires_a_session() {
        let (app, _conn) = test_app().await;
        let res = app.oneshot(walk_req(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// With a session it reaches the handler and returns a walk. Empty here because
    /// the test corpus has no recipes — an empty walk is a 200 with no stops, not
    /// an error (the walk reads whatever the corpus holds, even nothing).
    #[tokio::test]
    async fn walk_with_a_session_returns_a_walk_over_the_corpus() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(walk_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["stops"].as_array().expect("stops is an array").len(),
            0,
            "an empty corpus walks to nowhere"
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
            .oneshot(me_req(Some(&format!("recipes_session={token}"))))
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
            .oneshot(me_req(Some(&format!(
                "theme=dark; recipes_session={token}; lang=en"
            ))))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "the session must be found among other cookies"
        );
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

    /// Login cannot require a login: `complete` is reachable, and refuses an
    /// unknown secret rather than 401-ing for want of a session.
    #[tokio::test]
    async fn auth_complete_is_reachable_without_a_session() {
        let (app, _conn) = test_app().await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"c":"not-a-real-secret"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        // 401 because the secret is bogus — the endpoint itself was reachable.
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// **The regression test for the account takeover this design replaced.**
    ///
    /// The old flow let a browser start a login and keep a poll secret. An
    /// attacker started one, sent the link to a victim, and redeemed a session as
    /// them the moment they tapped — reproduced end-to-end before this rewrite.
    ///
    /// The fix is structural, so this asserts the structure: there is no endpoint
    /// through which anyone can *begin* a login and hold something that redeems
    /// it. The only way to a session is a secret the bot sent to a specific
    /// Telegram user's private chat.
    #[tokio::test]
    async fn no_endpoint_lets_a_caller_start_a_login_it_could_redeem() {
        let (app, _conn) = test_app().await;
        for path in ["/api/auth/start", "/api/auth/poll", "/api/auth/begin"] {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::NOT_FOUND,
                "{path} must not exist: a caller-initiated login is what let an \
                 attacker hand a victim a link and redeem their session"
            );
        }
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
