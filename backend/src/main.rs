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
//!   recipe-backend                          serve
//!   recipe-backend migrate                  apply pending DB migrations
//!   recipe-backend derive [<source>]        rebuild `recipes` from `raw_imports`
//!   recipe-backend enrich pull [--limit N]  GET the app's pending recipes (#59)
//!   recipe-backend enrich push              POST readings (from stdin) to the app
//!   recipe-backend steps pull [--limit N]   GET the app's pending methods (#74)
//!   recipe-backend steps push               POST step DAGs (from stdin) to the app
//!   recipe-backend mcp                       MCP stdio server: enrich/step pull/push tools

mod admin;
mod auth;
mod db;
mod derive;
mod enrich;
mod enrich_api;
mod error;
mod ingest;
mod kitchens;
mod mcp;
mod proxy;
mod recipes;
mod runs;
mod session;
mod step_api;
mod steps;
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
    /// The admin's Telegram id (`ADMIN_TELEGRAM_USER_ID`), gating the admin-only
    /// views (the health dashboard). `None` means no admin — the views 403 for
    /// everyone, fail-closed like the ingest key. See [`auth::is_admin`].
    pub admin_id: Option<String>,
    /// The live pick rooms (#20) — one `tokio::broadcast` channel per
    /// session. This is the only *stateful* part of the backend, and deliberately
    /// **not authoritative**: Turso holds every vote, so a lost process rehydrates
    /// on reconnect. See [`session`].
    pub rooms: session::Rooms,
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
        // The enrichment work queue (#59): a worker pulls the recipes still needing
        // a structured reading and pushes readings back. Machine-gated like
        // `/ingest` — the worker authenticates as infrastructure, and the app (never
        // the worker, never a model) is what writes the corpus.
        .route("/enrich/pending", get(enrich_api::pending))
        .route("/enrich/results", post(enrich_api::results))
        // The step-reading queue (#74/#75/#76): the same machine-gated shape for
        // reading a recipe's method into a step DAG.
        .route("/enrich/steps/pending", get(step_api::pending))
        .route("/enrich/steps/results", post(step_api::results))
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
        // Pick (#20): start a pick, then join its live room over a WS.
        // Both session-gated — the room needs to know who is voting, and joining is
        // never anonymous (#25).
        .route("/session", post(session::create))
        .route("/session/{channel}/ws", get(session::ws))
        // Admin-only health dashboard: session-gated here, then narrowed to the
        // configured admin inside the handler ([`admin::health`]).
        .route("/admin/health", get(admin::health))
        .route("/auth/logout", post(auth::logout))
        // Kitchens (#72): the durable shared space that scopes the meal flow. All
        // person-facing and session-gated; the handlers narrow to membership inside.
        .route("/kitchens", get(kitchens::list).post(kitchens::create))
        .route("/kitchens/join", post(kitchens::join))
        .route("/kitchens/{id}", get(kitchens::get))
        .route("/kitchens/{id}/name", post(kitchens::rename))
        .route(
            "/kitchens/{id}/equipment",
            post(kitchens::add_equipment).delete(kitchens::remove_equipment),
        )
        .route(
            "/kitchens/{id}/pantry",
            post(kitchens::add_pantry).delete(kitchens::remove_pantry),
        )
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
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
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

    // `recipe-backend mcp` — the enrichment MCP server (#59). Dispatched before the
    // default tracing init on purpose: that subscriber writes to stdout, and stdout
    // is the MCP JSON-RPC channel, so `mcp::serve` installs its own stderr
    // subscriber instead. Anything on stdout here would corrupt the protocol.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return mcp::serve().await;
    }

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
        // Open a run so this derive's writes are ordered against any concurrent
        // ingest, and close it with the outcome so a failed run is visible.
        let run_id = runs::begin(&conn, "derive").await?;
        let outcome = derive::derive(&conn, source.as_deref(), run_id).await;
        let status = if outcome.is_ok() {
            runs::COMPLETED
        } else {
            runs::FAILED
        };
        runs::finish(&conn, run_id, status).await?;
        let report = outcome?;
        tracing::info!(
            run_id,
            read = report.read,
            derived = report.derived,
            skipped = report.skipped,
            "derive complete"
        );
        return Ok(());
    }

    // `recipe-backend enrich pull|push` — the enrichment worker's two commands
    // (#59). They are HTTP clients for the app's machine-gated enrich endpoints, not
    // database access: the worker reads work and writes readings through the app's
    // front door (`RECIPES_API_URL` + `INGEST_API_KEY`), so the model behind the
    // enrich skill never touches the corpus. No model logic here either — the skill
    // does the reading; `push` only stamps `ENRICH_MODEL` and forwards. No DB, so
    // this opens no connection.
    //
    //   enrich pull [--limit N]  → GET the recipes still needing reading, to stdout
    //   enrich push              → POST readings read from stdin, print the result
    if std::env::args().nth(1).as_deref() == Some("enrich") {
        match std::env::args().nth(2).as_deref() {
            Some("pull") => {
                let args: Vec<String> = std::env::args().collect();
                let limit = args
                    .iter()
                    .position(|a| a == "--limit")
                    .and_then(|i| args.get(i + 1))
                    .and_then(|v| v.parse::<usize>().ok());
                enrich_api::client::pull(limit).await?;
            }
            Some("push") => enrich_api::client::push().await?,
            _ => {
                eprintln!(
                    "usage: recipe-backend enrich pull [--limit N] | recipe-backend enrich push"
                );
                std::process::exit(2);
            }
        }
        return Ok(());
    }

    // The step-reading queue's worker side (#74/#75/#76) — the same shape as `enrich`,
    // a different path. `steps pull` GETs the recipes still needing a step reading;
    // `steps push` POSTs the step DAGs read from stdin.
    if std::env::args().nth(1).as_deref() == Some("steps") {
        match std::env::args().nth(2).as_deref() {
            Some("pull") => {
                let args: Vec<String> = std::env::args().collect();
                let limit = args
                    .iter()
                    .position(|a| a == "--limit")
                    .and_then(|i| args.get(i + 1))
                    .and_then(|v| v.parse::<usize>().ok());
                step_api::client::pull(limit).await?;
            }
            Some("push") => step_api::client::push().await?,
            _ => {
                eprintln!(
                    "usage: recipe-backend steps pull [--limit N] | recipe-backend steps push"
                );
                std::process::exit(2);
            }
        }
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
        admin_id: auth::admin_id_from_env(),
        rooms: session::rooms(),
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
            // The test sessions below log in as "4242", so make that the admin.
            admin_id: Some("4242".into()),
            rooms: session::rooms(),
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

    fn enrich_pending_req(auth: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri("/api/enrich/pending");
        if let Some(v) = auth {
            b = b.header("authorization", v);
        }
        b.body(Body::empty()).unwrap()
    }

    fn enrich_results_req(auth: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/api/enrich/results")
            .header("content-type", "application/json");
        if let Some(v) = auth {
            b = b.header("authorization", v);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    }

    /// The enrich queue is machine-only, like ingest (#59): no key, no entry, and a
    /// session does not open it either — proving the new routes sit behind the
    /// machine gate, so a model reaching the app through them still can't get past
    /// the same door a browser can't.
    #[tokio::test]
    async fn enrich_endpoints_require_the_api_key_not_a_session() {
        let (app, conn) = test_app().await;
        for req in [
            enrich_pending_req(None),
            enrich_results_req(None, r#"{"model":"m","readings":[]}"#),
        ] {
            let res = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNAUTHORIZED,
                "no key must be refused"
            );
        }
        // A perfectly good session must not open a machine endpoint.
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/enrich/pending")
                    .header("cookie", format!("recipes_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "a session must not open the enrich queue"
        );
    }

    /// End to end through the router: a worker GETs pending, POSTs readings with the
    /// machine key, and the **app** stores + derives them. The caller only speaks
    /// HTTP+JSON — it never touches the database.
    #[tokio::test]
    async fn enrich_pending_then_results_round_trips_through_the_app() {
        let (app, conn) = test_app().await;
        conn.execute(
            "INSERT INTO raw_imports (source, id, raw, source_url) VALUES ('themealdb','1',?1,?2)",
            libsql::params![
                r#"{"meals":[{"idMeal":"1","strMeal":"T","strInstructions":"go","strIngredient1":"Flour","strMeasure1":"1 cup"}]}"#,
                "https://www.themealdb.com/api/json/v1/1/lookup.php?i=1"
            ],
        )
        .await
        .unwrap();
        derive::derive(&conn, None, 1).await.unwrap();

        let auth = "Bearer test-ingest-key";

        // pending lists the un-enriched recipe.
        let res = app
            .clone()
            .oneshot(enrich_pending_req(Some(auth)))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let pending: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pending.as_array().unwrap().len(), 1);
        assert_eq!(pending[0]["id"], "1");

        // push a matching reading.
        let submit = r#"{"model":"claude-opus-4-8","readings":[{"source":"themealdb","id":"1","readings":[{"item":"flour","amount":null,"preparation":null,"note":null}]}]}"#;
        let res = app
            .clone()
            .oneshot(enrich_results_req(Some(auth), submit))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(report["accepted"], 1);

        // pending is now empty — the recipe has a reading.
        let res = app.oneshot(enrich_pending_req(Some(auth))).await.unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let pending: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            pending.as_array().unwrap().len(),
            0,
            "reading stored → no longer pending"
        );
    }

    /// A blank model is a bad request, not a silently-stored placeholder (CodeRabbit).
    #[tokio::test]
    async fn enrich_results_rejects_a_blank_model() {
        let (app, _conn) = test_app().await;
        let res = app
            .oneshot(enrich_results_req(
                Some("Bearer test-ingest-key"),
                r#"{"model":"  ","readings":[]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    fn admin_health_req(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri("/api/admin/health");
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::empty()).unwrap()
    }

    /// The admin dashboard needs a session at all, like everything else.
    #[tokio::test]
    async fn admin_health_requires_a_session() {
        let (app, _conn) = test_app().await;
        let res = app.oneshot(admin_health_req(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// A logged-in NON-admin is refused — 403, not 401: the session is valid, the
    /// identity just is not the admin. `4242` is the configured admin (see the test
    /// state), so `9999` must not pass.
    #[tokio::test]
    async fn admin_health_forbids_a_non_admin() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "9999").await;
        let res = app
            .oneshot(admin_health_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// The admin passes the gate and gets the stats. The test corpus is empty, so
    /// the counts are 0 — what this pins is the gate + the response shape.
    #[tokio::test]
    async fn admin_health_serves_the_admin() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(admin_health_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        for key in [
            "recipes",
            "raw",
            "enriched",
            "enriched_pct",
            "by_model",
            "recent_runs",
            "running",
        ] {
            assert!(json.get(key).is_some(), "missing {key} in {json}");
        }
        assert_eq!(json["recipes"], 0, "empty test corpus");
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

    fn session_create_req(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/api/session")
            .header("content-type", "application/json");
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::from("{}")).unwrap()
    }

    /// Starting a pick is session-gated like the rest — joining is never
    /// anonymous (#25).
    #[tokio::test]
    async fn session_create_requires_a_session() {
        let (app, _conn) = test_app().await;
        let res = app.oneshot(session_create_req(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn session_create_with_a_session_mints_a_channel() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(session_create_req(Some(&format!(
                "recipes_session={token}"
            ))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
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
