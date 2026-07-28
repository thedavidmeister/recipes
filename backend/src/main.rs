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
mod boot;
mod db;
mod db_retry;
mod derive;
mod enrich;
mod enrich_api;
mod equipment;
mod equipment_api;
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
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::Response,
    routing::{get, post},
    Json, Router,
};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

use crate::error::AppError;

/// Shared handler state: the SSRF-guarded HTTP client, the Turso/libSQL database,
/// the Telegram config auth runs on, and the infra key that guards the ingest sync.
impl AppState {
    /// Run one logical database operation, retrying transient Turso failures.
    ///
    /// Every request-path DB touch goes through here, so the retry (#130) is
    /// inherited rather than sprinkled: `op` gets a connection, does its reads
    /// or writes, and returns; if the transport to Turso blips
    /// ([`db_retry::is_transient`]) the whole operation re-runs — bounded by
    /// [`db_retry::POLICY`] — and real errors (constraint violations, bad SQL)
    /// fail exactly as fast as they always did.
    ///
    /// **Each attempt gets a fresh connection**, and connections are deliberately
    /// per-operation rather than held for the process lifetime. A libsql
    /// connection owns a Hrana stream, and a stream does not survive the
    /// database changing generation — a restart or a failover on Turso's side.
    /// The long-lived connection we used to hold went stale exactly once and then
    /// failed *every* request after it with `stream not found: generation
    /// mismatch`, because nothing in the process ever asked for a new stream
    /// (#99). Connecting is cheap — it allocates a handle; the stream is
    /// established lazily on first use — so a fresh one per operation (and per
    /// retry: the stream that just broke is exactly what must not be reused)
    /// costs nothing.
    ///
    /// The closure must be safe to re-run. Reads trivially are; this app's
    /// writes are upserts, guarded (`run_id`) writes, deletes, or inserts keyed
    /// on values minted *outside* the closure — see #130 for the per-write
    /// audit. Mint ids/tokens before calling, not inside `op`, so every attempt
    /// writes the same row.
    pub async fn with_db<T, F, Fut>(&self, mut op: F) -> anyhow::Result<T>
    where
        F: FnMut(libsql::Connection) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        use futures_util::future::Either;
        db_retry::retry(db_retry::POLICY, |_attempt| match self.fresh_conn() {
            // `op` is called here, synchronously, so the future it returns is
            // its own value — independent of the borrow of `op` — which is what
            // keeps every handler's future provably `Send` (see
            // [`db_retry::retry`] for why this is not an `AsyncFnMut`).
            Ok(conn) => Either::Left(op(conn)),
            Err(e) => Either::Right(std::future::ready(Err(e))),
        })
        .await
    }

    /// One fresh connection, configured the way every caller assumes.
    fn fresh_conn(&self) -> anyhow::Result<libsql::Connection> {
        use anyhow::Context as _;
        let conn = self
            .database
            .connect()
            .context("could not reach the database")?;
        // Wait for a busy database rather than failing at once. Turso serves
        // the deployed app and has no file lock to contend for, but a local
        // file does — and SQLite's default is to give up immediately, so two
        // connections to a dev or test database would collide the moment one
        // of them wrote. Waiting is the behaviour every caller assumes.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .context("could not configure the database")?;
        Ok(conn)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub http: reqwest::Client,
    /// The database, **not** a connection. See [`AppState::db`].
    pub database: std::sync::Arc<libsql::Database>,
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
    /// Whether the schema is up yet (#146). Written by the boot task
    /// ([`boot::migrate_until_ready`]), read by [`require_schema`] and
    /// [`health`]. Boot no longer waits for the database, so this is how a
    /// request finds out whether there is anything to query.
    pub schema: boot::Readiness,
}

/// `GET /api/health` — the unauthenticated liveness probe, and the one route
/// that answers whatever the database is doing.
///
/// ```json
/// { "status": "ok",       "database": "ready"   }
/// { "status": "degraded", "database": "pending" }
/// { "status": "degraded", "database": "failed"  }
/// ```
///
/// **Always 200 while the process is alive**, deliberately. `render.yaml` points
/// `healthCheckPath` here, and Render will not promote a deploy whose health
/// check never passes — so answering 503 during a database outage would fail the
/// deploy and hand back the deploy freeze #146 exists to end. Liveness is the
/// status code; readiness is the body, where a prober and a human reading
/// Render's dashboard can both see it.
///
/// It carries no connection string, host, token, or error text: it is
/// unauthenticated by design, so it says *which* of three states we are in and
/// nothing about why. The why goes to the logs, where it is already gated.
async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let schema = state.schema.get();
    Json(serde_json::json!({
        "status": if schema.is_ready() { "ok" } else { "degraded" },
        "database": schema.as_str(),
    }))
}

/// Refuse a schema-dependent request while there is no schema to serve it (#146).
///
/// **503, not 500 and not a hang** — the same call `INGEST_API_KEY` makes: the
/// fault is the deployment's, not the caller's, so an operator reading it is
/// pointed at the deployment rather than sent hunting a credential. It answers
/// immediately; a request that waited for the database would just move the
/// outage into the client's timeout.
///
/// It sits **outside** the auth gates rather than behind them, because
/// [`auth::require_session`] reads the `sessions` table — with no schema it
/// cannot judge a cookie at all, so asking it first would turn every request
/// into a 500. The cost is that during the window an anonymous caller gets 503
/// where it would normally get 401. That leaks nothing (503 is not data, and no
/// route is reached), and it is the honest answer: we genuinely cannot say
/// whether that cookie is a session.
async fn require_schema(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    match state.schema.get() {
        boot::Schema::Ready => Ok(next.run(req).await),
        boot::Schema::Pending => Err(AppError::Unavailable(
            "the database schema is not ready yet; this service is still starting".into(),
        )),
        boot::Schema::Failed => Err(AppError::Unavailable(
            "the database schema could not be applied on this deployment".into(),
        )),
    }
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
        .route("/enrich/equipment/pending", get(equipment_api::pending))
        .route("/enrich/equipment/results", post(equipment_api::results))
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
        .route("/session/{channel}", get(session::lobby))
        .route("/session/{channel}/join", post(session::join_lobby))
        .route("/session/{channel}/start", post(session::start))
        .route("/session/{channel}/seat", post(session::seat))
        .route("/session/{channel}/meal-type", post(session::set_meal_type))
        .route("/session/{channel}/additions", post(session::set_additions))
        .route("/session/{channel}/cap", post(session::set_cap))
        // The meal's shopping checklist (#131) — read by anyone holding the channel
        // id (like the lobby it belongs to), written only by the people deciding it.
        .route(
            "/session/{channel}/buy",
            get(session::buy_list).post(session::set_buy_check),
        )
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
        .route("/kitchens/{id}/invite", post(kitchens::invite))
        // The vocabulary a kitchen picks from — a person's list, so session-gated.
        .route("/equipment", get(equipment_api::vocabulary))
        .route("/pantry", get(equipment_api::pantry_vocabulary))
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

    // Reachable without a session, each because requiring one would be circular
    // or wrong:
    //   /auth/complete    — redeems the bot's link; the secret in it IS the
    //                       authentication, and requiring a session to get one
    //                       would be circular.
    //   /telegram/webhook — called by Telegram, not a browser; it carries no
    //                       session and authenticates by its own secret instead.
    // Both still write the corpus's auth tables, so they need a schema like
    // everything else — public is not the same as schema-free.
    let public = Router::new()
        .route("/auth/complete", post(auth::complete))
        .route("/telegram/webhook", post(auth::webhook));

    // Everything that needs tables to answer, behind the readiness gate (#146).
    // The gate is applied here, outermost, so it runs before the auth
    // middlewares — see [`require_schema`] for why that order is forced.
    let schema_dependent = Router::new()
        .merge(machine)
        .merge(guarded)
        .merge(public)
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_schema,
        ));

    let api = Router::new()
        // The one route outside the gate: a liveness probe the host calls,
        // holding no session — and the thing that *reports* the gate, so it has
        // to answer when the database does not.
        .route("/health", get(health))
        .merge(schema_dependent)
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

    // The equipment queue's worker side (#81) — same shape again, another path.
    if std::env::args().nth(1).as_deref() == Some("equipment") {
        match std::env::args().nth(2).as_deref() {
            Some("pull") => {
                let args: Vec<String> = std::env::args().collect();
                let limit = args
                    .iter()
                    .position(|a| a == "--limit")
                    .and_then(|i| args.get(i + 1))
                    .and_then(|v| v.parse::<usize>().ok());
                equipment_api::client::pull(limit).await?;
            }
            Some("push") => equipment_api::client::push().await?,
            _ => {
                eprintln!(
                    "usage: recipe-backend equipment pull [--limit N] | recipe-backend equipment push"
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

    // Everything from here to `bind` is decided from configuration alone — no
    // packet is sent, so none of it can be a provider having a bad hour, and all
    // of it still refuses to boot (#146):
    //
    //   * `db::open` resolves `DATABASE_URL`/`TURSO_AUTH_TOKEN` into a handle. It
    //     does not connect. A placeholder or a bare path must die here rather
    //     than run beautifully against a container-local file (see `db.rs`).
    //   * Auth is mandatory, so missing Telegram config is a startup error: a
    //     backend that cannot mint a login can serve nothing.
    //
    // The ingest key is the exception — it gates one scheduled endpoint, so
    // missing it costs a sync, not the service. Warn and serve; ingest itself
    // refuses while it is unset.
    let database = db::open().await?;
    let ingest_key = auth::ingest_key_from_env();
    if ingest_key.is_none() {
        tracing::warn!("INGEST_API_KEY is not set — /api/ingest is disabled; the corpus will go stale until it is configured");
    }
    let state = AppState {
        http: proxy::build_client()?,
        database: std::sync::Arc::new(database),
        telegram: auth::TelegramConfig::from_env()?,
        cookie: auth::CookieConfig::from_env()?,
        ingest_key,
        admin_id: auth::admin_id_from_env(),
        rooms: session::rooms(),
        // Nothing has talked to the database yet. The boot task below decides.
        schema: boot::Readiness::pending(),
    };

    let app = app(state.clone());

    // Bind before touching the database, always. This is the line that unfroze
    // deploys: the process reaches "listening" whatever Turso is doing, so a blip
    // during a deploy — or during one of the free tier's constant cold starts —
    // costs some 503s instead of `Exited with status 1` and a live image nobody
    // can replace (#146).
    let addr: String = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("recipes backend listening on http://{addr}");

    // The schema comes up beside the server rather than in front of it. The first
    // attempt starts now, un-delayed, and on a healthy database it wins the race
    // with the first inbound request by a round trip; until it lands,
    // schema-dependent routes answer 503 and `/api/health` says `pending` — which
    // is strictly more than the old inline migration offered in the same window,
    // where the port was not bound and the connection was simply refused.
    tokio::spawn({
        let database = state.database.clone();
        let readiness = state.schema.clone();
        async move {
            boot::migrate_until_ready(boot::SCHEDULE, readiness, move |_attempt| {
                let database = database.clone();
                async move {
                    // A fresh connection per attempt: a libsql connection owns a
                    // Hrana stream, and the stream that just broke is exactly
                    // what must not be reused (#99).
                    let conn = database.connect()?;
                    db::migrate(&conn).await?;
                    // Expired rows are already refused on read, so this only
                    // reclaims space — a warning, never a reason to call the
                    // schema unready.
                    if let Err(e) = auth::sweep_expired(&conn).await {
                        tracing::warn!("could not sweep expired auth rows: {e}");
                    }
                    Ok(())
                }
            })
            .await;
        }
    });

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
        let (state, conn) = test_state(ingest_key).await;
        (app(state), conn)
    }

    /// The state itself, for tests that exercise [`AppState`] directly (the
    /// `with_db` retry) rather than through the router.
    async fn test_state(ingest_key: Option<String>) -> (AppState, libsql::Connection) {
        // A file rather than `:memory:`, because SQLite gives every connection to
        // `:memory:` its own private database — and the app now takes a fresh
        // connection per request, so an in-memory one would hand each request an empty
        // unmigrated database. A file is what production is: one database, many
        // connections. Unique per test so they cannot see each other.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "recipes-test-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let database = libsql::Builder::new_local(&path).build().await.unwrap();
        let conn = database.connect().unwrap();
        db::migrate(&conn).await.unwrap();
        // The returned connection is for the test's own assertions; the app takes its
        // own from the same database, exactly as it does in production.
        let state = AppState {
            http: proxy::build_client().unwrap(),
            database: std::sync::Arc::new(database),
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
            // Migrated a line above, so this is the healthy-boot state every
            // other test in this module assumes. The #146 tests flip it.
            schema: boot::Readiness::ready(),
        };
        (state, conn)
    }

    /// The retry composition end to end: a transient failure re-runs the closure,
    /// and the re-run holds a *working* connection to the same database — the
    /// fresh-connection-per-attempt half of #130 that the `db_retry` unit tests
    /// cannot see.
    #[tokio::test]
    async fn with_db_rides_out_a_transient_failure_on_a_fresh_connection() {
        let (state, _conn) = test_state(None).await;
        let failures_left = std::cell::Cell::new(2u32);
        let failures_left = &failures_left;
        let answer = state
            .with_db(move |conn| async move {
                // Prove every attempt's connection actually works before the
                // injected failure decides this attempt's fate.
                let mut rows = conn.query("SELECT 41 + 1", ()).await?;
                let value = rows
                    .next()
                    .await?
                    .expect("one row")
                    .get::<i64>(0)
                    .expect("one column");
                if failures_left.get() > 0 {
                    failures_left.set(failures_left.get() - 1);
                    // The incident error, verbatim (#130) — transient.
                    return Err(libsql::Error::Hrana(
                        r#"api error: `status=502 Bad Gateway, body={"error":"upstream forward failed"}`"#
                            .to_string()
                            .into(),
                    )
                    .into());
                }
                Ok(value)
            })
            .await
            .expect("two blips then success must succeed");
        assert_eq!(answer, 42);
        assert_eq!(
            failures_left.get(),
            0,
            "both injected failures were consumed"
        );
    }

    /// The other half of the bound: a fatal error is never retried, so a real
    /// database ruling costs exactly one attempt.
    #[tokio::test]
    async fn with_db_does_not_retry_a_database_ruling() {
        let (state, _conn) = test_state(None).await;
        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        let result = state
            .with_db(move |conn| async move {
                calls.set(calls.get() + 1);
                conn.execute("NOT EVEN SQL", ()).await?;
                Ok(())
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            calls.get(),
            1,
            "malformed SQL must fail on the first attempt"
        );
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
    fn json_post(uri: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(v) = cookie {
            b = b.header("cookie", v);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    }

    async fn body_json(res: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Make a kitchen through the API as `who`, returning its id — the same path a
    /// person takes, so these tests exercise the router rather than a lookalike.
    async fn make_kitchen(app: &Router, cookie: &str, name: &str) -> String {
        let res = app
            .clone()
            .oneshot(json_post(
                "/api/kitchens",
                Some(cookie),
                &format!(r#"{{"name":"{name}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["id"].as_str().unwrap().to_owned()
    }

    /// Start a meal plan in `kitchen`, as `cookie`, and return its channel.
    async fn make_plan(app: &Router, cookie: &str, kitchen: &str) -> String {
        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(cookie),
                &format!(r#"{{"kitchen_id":"{kitchen}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Seating someone into a plan is gated like everything a person reaches (#25).
    #[tokio::test]
    async fn seating_requires_a_session() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/seat"),
                None,
                r#"{"user_id":"mel"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// Only the host curates the roster — a guest cannot add people to someone else's
    /// plan.
    #[tokio::test]
    async fn only_the_host_can_seat() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let other = auth::issue_test_session(&conn, "other").await;
        let hcookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &hcookie, "Home").await;
        let channel = make_plan(&app, &hcookie, &kid).await;

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/seat"),
                Some(&format!("recipes_session={other}")),
                r#"{"user_id":"mel"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// The seatable pool is exactly the kitchen's members — the host cannot pull in a
    /// stranger, only offer the link.
    #[tokio::test]
    async fn seating_a_non_member_is_refused() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/seat"),
                Some(&cookie),
                r#"{"user_id":"a-stranger"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// A plan is born for dinner (#114): name no meal at creation and the lobby says
    /// so. The host then repoints it, and the answer — a full lobby view — carries the
    /// new meal for every screen to agree on.
    #[tokio::test]
    async fn the_host_names_the_meal_and_the_lobby_carries_it() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/session/{channel}"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        assert_eq!(body["meal_type"], "dinner");
        assert_eq!(body["additions"], serde_json::json!([]), "a plain meal");

        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/meal-type"),
                Some(&cookie),
                r#"{"meal_type":"breakfast"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await["meal_type"], "breakfast");

        // And what comes with it — replied in canonical order, however the host's
        // client happened to say it.
        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/additions"),
                Some(&cookie),
                r#"{"additions":["drink","dessert"]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            body_json(res).await["additions"],
            serde_json::json!(["dessert", "drink"])
        );
    }

    /// A plan can be created *for* a meal — the API takes the type up front, so a
    /// future create flow that asks first needs no second request.
    #[tokio::test]
    async fn a_plan_can_be_created_for_a_meal() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");

        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(&cookie),
                r#"{"meal_type":"snack","additions":["drink"]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/session/{channel}"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_json(res).await;
        assert_eq!(body["meal_type"], "snack");
        assert_eq!(body["additions"], serde_json::json!(["drink"]));
    }

    /// The vocabulary is closed server-side: a meal outside it is refused at the
    /// wire, on create and on change alike. "dessert" is deliberate: it is an
    /// addition *to* a meal, not a meal you sit down to, so it is as refused as a
    /// made-up word.
    #[tokio::test]
    async fn a_meal_outside_the_vocabulary_is_refused() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(&cookie),
                r#"{"meal_type":"dessert"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/meal-type"),
                Some(&cookie),
                r#"{"meal_type":"brunch"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Only the host names the meal — a guest cannot repoint someone else's plan.
    #[tokio::test]
    async fn only_the_host_can_name_the_meal() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let other = auth::issue_test_session(&conn, "other").await;
        let hcookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &hcookie, "Home").await;
        let channel = make_plan(&app, &hcookie, &kid).await;

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/meal-type"),
                Some(&format!("recipes_session={other}")),
                r#"{"meal_type":"lunch"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// Once the swiping starts the meal is fixed: people voted on *that* meal, so
    /// even the host cannot move it under them.
    #[tokio::test]
    async fn a_started_plan_keeps_its_meal() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/start"),
                Some(&cookie),
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/meal-type"),
                Some(&cookie),
                r#"{"meal_type":"lunch"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// Minting an invite is gated like everything else a person reaches (#25).
    #[tokio::test]
    async fn minting_an_invite_requires_a_session() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        let id = make_kitchen(&app, &cookie, "Home").await;

        let res = app
            .oneshot(json_post(&format!("/api/kitchens/{id}/invite"), None, "{}"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// The boundary that matters: a stranger cannot mint themselves a way in. Without
    /// this, an invite endpoint would be an open door to any kitchen whose id leaked —
    /// and since everyone in a kitchen is an owner of it, that is the whole kitchen.
    #[tokio::test]
    async fn a_stranger_cannot_mint_an_invite_to_your_kitchen() {
        let (app, conn) = test_app().await;
        let mine = auth::issue_test_session(&conn, "4242").await;
        let theirs = auth::issue_test_session(&conn, "9317").await;
        let id = make_kitchen(&app, &format!("recipes_session={mine}"), "Home").await;

        let res = app
            .oneshot(json_post(
                &format!("/api/kitchens/{id}/invite"),
                Some(&format!("recipes_session={theirs}")),
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// A member gets a token, and one that dies: the response says when, and it says a
    /// time roughly two hours out rather than never.
    #[tokio::test]
    async fn a_member_gets_an_invite_that_expires() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        let id = make_kitchen(&app, &cookie, "Home").await;

        let res = app
            .oneshot(json_post(
                &format!("/api/kitchens/{id}/invite"),
                Some(&cookie),
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = body_json(res).await;
        let minted = body["token"].as_str().unwrap();
        assert!(!minted.is_empty(), "a token to put in a link");

        let expires_at = body["expires_at"].as_i64().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let two_hours = 2 * 60 * 60;
        assert!(
            expires_at > now && expires_at <= now + two_hours + 5,
            "expires about two hours out, not never: {expires_at} vs {now}"
        );
    }

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
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        eprintln!("DEBUG body: {}", String::from_utf8_lossy(&body));
        assert_eq!(status, StatusCode::OK);
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

    /// A `GET` with a cookie, for the session/lobby and walk reads below.
    fn get_req(uri: &str, cookie: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    }

    /// A plan created with a time cap (#80) shows it in the lobby view, so everyone
    /// sees the bound they will be swiping within — whatever number was asked for,
    /// not just the buckets the lobby offers.
    #[tokio::test]
    async fn a_plan_created_with_a_cap_shows_it_in_the_lobby() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");

        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(&cookie),
                r#"{"max_total_seconds":1800}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let res = app
            .clone()
            .oneshot(get_req(&format!("/api/session/{channel}"), &cookie))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await["max_total_seconds"], 1800);

        // A cap the buckets do not offer is still a cap: the API takes seconds, not
        // the UI's vocabulary (#80).
        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(&cookie),
                r#"{"max_total_seconds":5400}"#,
            ))
            .await
            .unwrap();
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let res = app
            .oneshot(get_req(&format!("/api/session/{channel}"), &cookie))
            .await
            .unwrap();
        assert_eq!(body_json(res).await["max_total_seconds"], 5400);
    }

    /// A plan is born capped at half an hour (#163), and a caller can still say
    /// otherwise.
    ///
    /// The two halves are one test because they are one decision. A body that names
    /// no cap gets 1800: the lobby's time row then starts on a setting that filters
    /// something, instead of sitting inert on the one option that filters nothing.
    /// A body that says `null` gets "Any": the default is where a plan starts, not a
    /// floor, and `null` is the same word that lifts the cap in the lobby — so
    /// absent and `null` must not collapse into each other, which is the one way
    /// this could quietly become unliftable.
    #[tokio::test]
    async fn a_plan_is_born_capped_at_thirty_minutes() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");

        let cap_of = |app: axum::Router, body: &'static str, cookie: String| async move {
            let res = app
                .clone()
                .oneshot(json_post("/api/session", Some(&cookie), body))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{body} must create a plan");
            let channel = body_json(res).await["channel_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let res = app
                .oneshot(get_req(&format!("/api/session/{channel}"), &cookie))
                .await
                .unwrap();
            body_json(res).await["max_total_seconds"].clone()
        };

        // Nothing said at all, and everything-but-the-cap said: both are "a plan,
        // please", and both are born at 30 minutes.
        assert_eq!(cap_of(app.clone(), "{}", cookie.clone()).await, 1800);
        assert_eq!(
            cap_of(app.clone(), r#"{"meal_type":"lunch"}"#, cookie.clone()).await,
            1800
        );

        // "Any", said out loud, is still unbounded.
        assert!(
            cap_of(app.clone(), r#"{"max_total_seconds":null}"#, cookie.clone())
                .await
                .is_null()
        );
    }

    /// The walk a born-capped plan deals is bounded by that default, without anyone
    /// touching the control (#163) — the point of the default is that it does
    /// something before it is looked at. The lower-bound policy still holds inside
    /// it: an un-estimated recipe stays (#80, #158).
    #[tokio::test]
    async fn a_born_capped_plan_walks_within_its_default() {
        let (app, conn) = test_app().await;
        for (id, secs) in [
            ("quick", Some(900i64)),
            ("slow", Some(3600i64)),
            ("unknown", None),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, total_seconds)
                 VALUES ('t', ?1, ?1, ?2)",
                libsql::params![id, secs],
            )
            .await
            .unwrap();
        }
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");

        let res = app
            .clone()
            .oneshot(json_post("/api/session", Some(&cookie), "{}"))
            .await
            .unwrap();
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let res = app
            .oneshot(get_req(
                &format!("/api/walk?len=10&channel={channel}"),
                &cookie,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let stops = body_json(res).await["stops"].clone();
        let ids: std::collections::HashSet<String> = stops
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["recipe"]["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains("quick"), "under the default stays: {ids:?}");
        assert!(ids.contains("unknown"), "no estimate stays: {ids:?}");
        assert!(
            !ids.contains("slow"),
            "an hour is over the default and goes: {ids:?}"
        );
    }

    /// A nonsense cap — zero, negative, longer than a day — is refused at create.
    #[tokio::test]
    async fn a_plan_with_a_nonsense_cap_is_refused() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        for body in [
            r#"{"max_total_seconds":0}"#,
            r#"{"max_total_seconds":-60}"#,
            r#"{"max_total_seconds":90000}"#,
        ] {
            let res = app
                .clone()
                .oneshot(json_post("/api/session", Some(&cookie), body))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::BAD_REQUEST,
                "{body} must be refused"
            );
        }
    }

    /// The cap is the host's to move, and only while the lobby is open: a guest
    /// cannot rebound someone else's plan, and once the swiping starts the corpus
    /// everyone is voting within must not shift under them (#80).
    #[tokio::test]
    async fn the_cap_is_the_hosts_and_freezes_at_start() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let guest = auth::issue_test_session(&conn, "guest").await;
        let hcookie = format!("recipes_session={host}");
        let gcookie = format!("recipes_session={guest}");

        let res = app
            .clone()
            .oneshot(json_post("/api/session", Some(&hcookie), "{}"))
            .await
            .unwrap();
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let cap_uri = format!("/api/session/{channel}/cap");

        // Not the host → forbidden.
        let res = app
            .clone()
            .oneshot(json_post(
                &cap_uri,
                Some(&gcookie),
                r#"{"max_total_seconds":1800}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // The host, lobby open → the cap moves off the default it was born with
        // (#163) and the new lobby comes back.
        let res = app
            .clone()
            .oneshot(json_post(
                &cap_uri,
                Some(&hcookie),
                r#"{"max_total_seconds":3600}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await["max_total_seconds"], 3600);

        // A nonsense cap is refused here too.
        let res = app
            .clone()
            .oneshot(json_post(
                &cap_uri,
                Some(&hcookie),
                r#"{"max_total_seconds":-1}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // Start the plan; the cap is now frozen, even for the host.
        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/start"),
                Some(&hcookie),
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = app
            .oneshot(json_post(
                &cap_uri,
                Some(&hcookie),
                r#"{"max_total_seconds":7200}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// End to end through the router (#80): the walk a capped session deals
    /// excludes recipes estimated over the cap, keeps under-cap ones, and keeps
    /// the un-estimated (`NULL`) — the lower-bound policy (see `walk::load_corpus`).
    /// The same corpus walked without a channel is the whole corpus.
    #[tokio::test]
    async fn a_capped_sessions_walk_excludes_over_cap_recipes() {
        let (app, conn) = test_app().await;
        for (id, secs) in [
            ("quick", Some(900i64)),
            ("slow", Some(7200i64)),
            ("unknown", None),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, total_seconds)
                 VALUES ('t', ?1, ?1, ?2)",
                libsql::params![id, secs],
            )
            .await
            .unwrap();
        }
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(&cookie),
                r#"{"max_total_seconds":1800}"#,
            ))
            .await
            .unwrap();
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let res = app
            .clone()
            .oneshot(get_req(
                &format!("/api/walk?len=10&channel={channel}"),
                &cookie,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let stops = body_json(res).await["stops"].clone();
        let ids: std::collections::HashSet<String> = stops
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["recipe"]["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains("quick"), "under the cap stays: {ids:?}");
        assert!(ids.contains("unknown"), "no estimate stays: {ids:?}");
        assert!(!ids.contains("slow"), "over the cap is excluded: {ids:?}");

        // The same walk without a channel wanders the whole corpus.
        let res = app
            .oneshot(get_req("/api/walk?len=10", &cookie))
            .await
            .unwrap();
        let stops = body_json(res).await["stops"].clone();
        assert_eq!(stops.as_array().unwrap().len(), 3);
    }

    /// A kitchen holding `items`, and a plan for it, end to end through the router —
    /// returns the plan's channel id (#82).
    async fn kitchen_plan(
        app: &Router,
        conn: &libsql::Connection,
        cookie: &str,
        items: &[&str],
    ) -> String {
        let res = app
            .clone()
            .oneshot(json_post(
                "/api/kitchens",
                Some(cookie),
                r#"{"name":"Home"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let kitchen = body_json(res).await["id"].as_str().unwrap().to_owned();
        // Straight in, because `POST /kitchens/{id}/equipment` only admits names the
        // corpus asks for and this fixture's corpus is written directly too.
        for item in items {
            conn.execute(
                "INSERT INTO kitchen_equipment (kitchen_id, item) VALUES (?1, ?2)",
                libsql::params![kitchen.clone(), *item],
            )
            .await
            .unwrap();
        }
        let res = app
            .clone()
            .oneshot(json_post(
                "/api/session",
                Some(cookie),
                &format!(r#"{{"kitchen_id":"{kitchen}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The ids a walk over `channel` deals.
    async fn walked(
        app: &Router,
        cookie: &str,
        channel: &str,
    ) -> std::collections::HashSet<String> {
        let res = app
            .clone()
            .oneshot(get_req(
                &format!("/api/walk?len=10&channel={channel}"),
                cookie,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["recipe"]["id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Three recipes: one this kitchen can make, one needing a tool it lacks, one
    /// nobody has read for equipment.
    async fn seed_equipment_corpus(conn: &libsql::Connection) {
        for (id, equipment) in [
            ("simple", r#"[{"item":"knife"}]"#),
            ("blender", r#"[{"item":"knife"},{"item":"blender"}]"#),
            ("unread", "[]"),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, equipment) VALUES ('t', ?1, ?1, ?2)",
                libsql::params![id, equipment],
            )
            .await
            .unwrap();
        }
    }

    /// End to end through the router (#82): a plan for a kitchen deals only recipes
    /// that kitchen has every tool for — no flag, nothing to turn on. A recipe needing
    /// a tool it lacks never appears, and neither does one nobody has read for
    /// equipment, which cannot be proven makeable either way.
    #[tokio::test]
    async fn a_plans_walk_deals_only_what_its_kitchen_can_make() {
        let (app, conn) = test_app().await;
        seed_equipment_corpus(&conn).await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        let channel = kitchen_plan(&app, &conn, &cookie, &["knife"]).await;

        let ids = walked(&app, &cookie, &channel).await;
        assert!(ids.contains("simple"), "every tool owned: {ids:?}");
        assert!(!ids.contains("blender"), "one tool short: {ids:?}");
        assert!(!ids.contains("unread"), "not proven makeable: {ids:?}");
    }

    /// The exception, and the reason it exists (#82): a kitchen with **nothing
    /// recorded** limits nothing, because that is a gap in what we know and not a claim
    /// that it owns no tools — the same ruling #81 made when it refused an empty
    /// reading. Were zero read as a claim, every real user would get an empty pick: the
    /// only kitchen in production holds zero items.
    ///
    /// Stocking the kitchen brings the limit into force on the very next walk, with no
    /// new plan and nothing to switch on.
    #[tokio::test]
    async fn a_kitchen_with_nothing_recorded_limits_nothing() {
        let (app, conn) = test_app().await;
        seed_equipment_corpus(&conn).await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");

        let bare = kitchen_plan(&app, &conn, &cookie, &[]).await;
        assert_eq!(
            walked(&app, &cookie, &bare).await.len(),
            3,
            "unknown equipment must not empty the deck"
        );

        // Record one thing, and the same plan narrows immediately.
        let kitchen = body_json(
            app.clone()
                .oneshot(get_req(&format!("/api/session/{bare}"), &cookie))
                .await
                .unwrap(),
        )
        .await["kitchen_id"]
            .as_str()
            .unwrap()
            .to_owned();
        conn.execute(
            "INSERT INTO kitchen_equipment (kitchen_id, item) VALUES (?1, 'knife')",
            libsql::params![kitchen],
        )
        .await
        .unwrap();

        let ids = walked(&app, &cookie, &bare).await;
        assert_eq!(ids.len(), 1, "now limited to what it can make: {ids:?}");
        assert!(ids.contains("simple"));
    }

    /// A plan started outside a kitchen has nothing to match against, so it walks the
    /// whole corpus — the same gap, reached a different way.
    #[tokio::test]
    async fn a_kitchenless_plan_walks_everything() {
        let (app, conn) = test_app().await;
        seed_equipment_corpus(&conn).await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        let res = app
            .clone()
            .oneshot(json_post("/api/session", Some(&cookie), "{}"))
            .await
            .unwrap();
        let channel = body_json(res).await["channel_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(walked(&app, &cookie, &channel).await.len(), 3);
    }

    /// A walk naming a session that does not exist is refused — never silently
    /// walked uncapped, which would hand a mistyped channel the whole corpus.
    #[tokio::test]
    async fn a_walk_for_an_unknown_session_is_refused() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = app
            .oneshot(get_req(
                "/api/walk?channel=nope",
                &format!("recipes_session={token}"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
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

    // ---- Boot degrades, it does not die (#146) -----------------------------

    /// `GET /api/health`, asserting the status code the deploy depends on and
    /// returning the body that carries the truth.
    ///
    /// The 200 is the load-bearing part: `render.yaml` points `healthCheckPath`
    /// here, and a deploy whose health check never passes is a deploy that never
    /// promotes — which is the freeze this whole feature exists to end. Liveness
    /// is the code; readiness is the body.
    async fn health_body(router: &Router) -> serde_json::Value {
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "/api/health must answer 200 while the process is alive — Render gates the deploy on it"
        );
        body_json(res).await
    }

    /// One request per gate the router has, so "everything that needs tables"
    /// is probed rather than asserted: a session route, a machine route, and the
    /// two unauthenticated endpoints that still read and write the auth tables.
    /// Public is not the same as schema-free.
    fn schema_dependent_probes() -> Vec<Request<Body>> {
        vec![
            me_req(None),
            walk_req(None),
            get_req("/api/kitchens", "recipes_session=whatever"),
            // With a *valid* key, so what answers is the gate rather than the
            // machine auth — and so no real sync can run behind it.
            ingest_req(Some("Bearer test-ingest-key"), None),
            enrich_pending_req(Some("Bearer test-ingest-key")),
            Request::builder()
                .method("POST")
                .uri("/api/auth/complete")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"c":"a-secret"}"#))
                .unwrap(),
            // The right webhook secret, again so the gate is what answers.
            Request::builder()
                .method("POST")
                .uri("/api/telegram/webhook")
                .header("content-type", "application/json")
                .header("x-telegram-bot-api-secret-token", "test-webhook-secret")
                .body(Body::from(
                    r#"{"message":{"text":"/start abc","from":{"id":1}}}"#,
                ))
                .unwrap(),
        ]
    }

    /// Attempts land 1ms apart so a test does not sit through the real schedule.
    const TEST_SCHEDULE: boot::Schedule = boot::Schedule {
        base: std::time::Duration::from_millis(1),
        cap: std::time::Duration::from_millis(1),
    };

    /// The boot error that killed four consecutive deploys on 2026-07-25,
    /// verbatim.
    fn boot_incident_error() -> anyhow::Error {
        libsql::Error::Hrana(
            "cursor error: `error reading a body from connection: unexpected EOF during chunk size line`"
                .to_string()
                .into(),
        )
        .into()
    }

    /// **The headline (#146)**, through the real router.
    ///
    /// A transient database failure at boot does not end the process: it binds,
    /// it serves, schema-dependent routes answer an honest 503, and `/api/health`
    /// distinguishes "up, database unreachable" from a dead process. Then the
    /// migration lands on a later attempt and the whole surface flips to ready —
    /// no redeploy, no human.
    ///
    /// The failure is injected at the boot task's own seam (the attempt closure,
    /// which is where `main` puts `connect` + `migrate`), so the schedule, the
    /// classification and the readiness flip are the *real* ones over the *real*
    /// database — only the transport failure is faked, because faking Hrana would
    /// be faking the thing under test.
    #[tokio::test]
    async fn a_transient_boot_failure_degrades_and_then_recovers() {
        let (state, _conn) = test_state(Some("test-ingest-key".into())).await;
        // The state the process is in the instant it binds its port: nothing has
        // talked to the database yet.
        state.schema.set(boot::Schema::Pending);
        let router = app(state.clone());

        // Up — and saying which kind of up.
        assert_eq!(
            health_body(&router).await,
            serde_json::json!({"status": "degraded", "database": "pending"}),
            "a prober must be able to tell a degraded process from a dead one"
        );

        // Every route that needs tables answers 503. Not 500, not a hang, not a
        // wrong answer from an empty database.
        for req in schema_dependent_probes() {
            let (method, uri) = (req.method().clone(), req.uri().clone());
            let res = router.clone().oneshot(req).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{method} {uri} must answer 503 while the schema is not up"
            );
        }

        // Turso comes back on the third attempt. This is the same task `main`
        // spawns, over the same database — the first two attempts fail in
        // transport, the third really migrates.
        let database = state.database.clone();
        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        boot::migrate_until_ready(TEST_SCHEDULE, state.schema.clone(), move |_attempt| {
            let database = database.clone();
            async move {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    return Err(boot_incident_error());
                }
                let conn = database.connect()?;
                db::migrate(&conn).await?;
                Ok(())
            }
        })
        .await;
        assert_eq!(calls.get(), 3, "two blips, then the migration lands");

        assert_eq!(
            health_body(&router).await,
            serde_json::json!({"status": "ok", "database": "ready"}),
            "recovery is reported, not merely achieved"
        );

        // And the gate is out of the way again: auth answers for itself, so the
        // degraded window did not leave anything wedged — or open.
        let res = router.clone().oneshot(me_req(None)).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "auth is mandatory again the moment the schema is up"
        );
        let token = auth::issue_test_session(&state.database.connect().unwrap(), "4242").await;
        let res = router
            .oneshot(me_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "a real session reaches the handler once the schema is up"
        );
    }

    /// A ruling — a credential the database refused, SQL it rejected — is
    /// **permanently unready**, not an exit.
    ///
    /// It is one attempt (retrying collects the same ruling), it is loud in the
    /// logs, and it is visible: `/api/health` says `failed` rather than `pending`,
    /// so an operator can tell "wait, it is coming back" from "this needs me".
    /// The process stays up precisely so that answer can still be asked for; a
    /// container that exited answers nothing, and exiting on a verdict the
    /// *provider* worded would re-arm the deploy freeze.
    #[tokio::test]
    async fn a_fatal_boot_failure_stays_up_and_reports_failed() {
        let (state, _conn) = test_state(None).await;
        state.schema.set(boot::Schema::Pending);
        let router = app(state.clone());

        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        boot::migrate_until_ready(
            TEST_SCHEDULE,
            state.schema.clone(),
            move |_attempt| async move {
                calls.set(calls.get() + 1);
                // A remote statement error: the database judged this and said no.
                Err(anyhow::Error::from(libsql::Error::Hrana(
                    r#"api error: `status=401 Unauthorized, body=`"#.to_string().into(),
                )))
            },
        )
        .await;
        assert_eq!(calls.get(), 1, "a ruling must not be retried");

        assert_eq!(
            health_body(&router).await,
            serde_json::json!({"status": "degraded", "database": "failed"}),
            "`failed` and `pending` must be tellable apart — one resolves itself, one does not"
        );
        for req in schema_dependent_probes() {
            let uri = req.uri().clone();
            let res = router.clone().oneshot(req).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{uri} must answer 503, not 500, on a deployment whose schema will not come up"
            );
        }
    }

    /// A healthy boot is unchanged, and now says so: the schema is ready, the
    /// gate is transparent, and `/api/health` reports both.
    #[tokio::test]
    async fn a_healthy_boot_reports_ready_and_gates_nothing() {
        let (state, conn) = test_state(Some("test-ingest-key".into())).await;
        let router = app(state);

        assert_eq!(
            health_body(&router).await,
            serde_json::json!({"status": "ok", "database": "ready"})
        );

        // The gate does not touch a ready service: the answers are the routes'
        // own, exactly as before #146.
        let res = router.clone().oneshot(me_req(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "auth, not the gate");
        let token = auth::issue_test_session(&conn, "4242").await;
        let res = router
            .oneshot(me_req(Some(&format!("recipes_session={token}"))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// The degraded service must not become a *permissive* one: no route that
    /// needs a credential when healthy answers without one when degraded. The
    /// gate only ever subtracts — it refuses before any handler, so there is no
    /// path on which "the database is down" turns into data.
    #[tokio::test]
    async fn a_degraded_service_never_serves_more_than_a_healthy_one() {
        let (state, _conn) = test_state(Some("test-ingest-key".into())).await;
        state.schema.set(boot::Schema::Pending);
        let router = app(state);

        for req in [
            // No credential at all, on each kind of gate.
            ingest_req(None, None),
            enrich_pending_req(None),
            me_req(None),
            // A forged webhook, which a healthy service refuses on its secret.
            Request::builder()
                .method("POST")
                .uri("/api/telegram/webhook")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":{"text":"/start abc","from":{"id":1,"username":"mallory"}}}"#,
                ))
                .unwrap(),
        ] {
            let uri = req.uri().clone();
            let res = router.clone().oneshot(req).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{uri} must be refused while degraded, never served"
            );
        }
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

    // ---- buy checklist (#131) ----------------------------------------------

    /// The meal's shopping list is a person-facing surface, so both halves of it are
    /// session-gated like the rest (#25).
    #[tokio::test]
    async fn the_buy_checklist_requires_a_session() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/session/{channel}/buy?source=t&id=1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/buy"),
                None,
                r#"{"source":"t","id":"1","index":0,"checked":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// End to end through the router (#131): a decider ticks a line, it comes back
    /// attributed to them, a second decider takes it over, and an untick clears it.
    #[tokio::test]
    async fn a_tick_round_trips_through_the_router_carrying_who_ticked_it() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let host_cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &host_cookie, "Home").await;
        let channel = make_plan(&app, &host_cookie, &kid).await;

        // A second decider, seated by the host the way the lobby does it (#72).
        kitchens::seat_member_for_test(&conn, &kid, "mel").await;
        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/seat"),
                Some(&host_cookie),
                r#"{"user_id":"mel"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let mel = auth::issue_test_session(&conn, "mel").await;
        let mel_cookie = format!("recipes_session={mel}");
        // Logging in registers the person; the handle comes from Telegram after, so
        // it is set here rather than before (`upsert_user` writes what it was told,
        // which for a test login is no handle at all).
        conn.execute(
            "UPDATE users SET username = 'mel' WHERE telegram_user_id = 'mel'",
            (),
        )
        .await
        .unwrap();

        // Nothing is in the basket yet.
        let res = app
            .clone()
            .oneshot(get_req(
                &format!("/api/session/{channel}/buy?source=themealdb&id=52772"),
                &host_cookie,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        assert_eq!(body["channel_id"], channel);
        assert_eq!(body["source"], "themealdb");
        assert_eq!(body["checks"].as_array().unwrap().len(), 0);

        // The host grabs the flour.
        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/buy"),
                Some(&host_cookie),
                r#"{"source":"themealdb","id":"52772","index":1,"checked":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let checks = body_json(res).await["checks"].clone();
        assert_eq!(checks[0]["index"], 1);
        assert_eq!(checks[0]["by"]["telegram_user_id"], "host");

        // Mel had already picked it up — last writer wins, and there is still one row.
        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/buy"),
                Some(&mel_cookie),
                r#"{"source":"themealdb","id":"52772","index":1,"checked":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let checks = body_json(res).await["checks"].clone();
        assert_eq!(checks.as_array().unwrap().len(), 1);
        assert_eq!(checks[0]["by"]["telegram_user_id"], "mel");
        assert_eq!(checks[0]["by"]["username"], "mel");

        // Put it back.
        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/buy"),
                Some(&host_cookie),
                r#"{"source":"themealdb","id":"52772","index":1,"checked":false}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await["checks"].as_array().unwrap().len(), 0);
    }

    /// A signed-in stranger holding the channel id must not be able to write into
    /// someone else's basket — the roster is who is having this meal (#131). 403,
    /// not 401: the session is fine, the identity just is not on the list.
    #[tokio::test]
    async fn a_non_member_cannot_tick_someone_elses_list() {
        let (app, conn) = test_app().await;
        let host = auth::issue_test_session(&conn, "host").await;
        let host_cookie = format!("recipes_session={host}");
        let kid = make_kitchen(&app, &host_cookie, "Home").await;
        let channel = make_plan(&app, &host_cookie, &kid).await;

        let stranger = auth::issue_test_session(&conn, "stranger").await;
        let res = app
            .clone()
            .oneshot(json_post(
                &format!("/api/session/{channel}/buy"),
                Some(&format!("recipes_session={stranger}")),
                r#"{"source":"themealdb","id":"52772","index":0,"checked":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // And the refusal is real: nothing was written.
        let res = app
            .oneshot(get_req(
                &format!("/api/session/{channel}/buy?source=themealdb&id=52772"),
                &host_cookie,
            ))
            .await
            .unwrap();
        assert_eq!(body_json(res).await["checks"].as_array().unwrap().len(), 0);
    }

    /// A checklist for a channel that does not exist is refused on both verbs — a
    /// mistyped channel must never conjure a room, the same rule the WS upgrade and
    /// the walk already hold to.
    #[tokio::test]
    async fn a_checklist_for_an_unknown_session_is_refused() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");

        let res = app
            .clone()
            .oneshot(get_req("/api/session/nope/buy?source=t&id=1", &cookie))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let res = app
            .oneshot(json_post(
                "/api/session/nope/buy",
                Some(&cookie),
                r#"{"source":"t","id":"1","index":0,"checked":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// A negative ingredient index names no line of any recipe, so it is an author
    /// error rather than a row to write.
    #[tokio::test]
    async fn a_negative_ingredient_index_is_refused() {
        let (app, conn) = test_app().await;
        let token = auth::issue_test_session(&conn, "4242").await;
        let cookie = format!("recipes_session={token}");
        let kid = make_kitchen(&app, &cookie, "Home").await;
        let channel = make_plan(&app, &cookie, &kid).await;

        let res = app
            .oneshot(json_post(
                &format!("/api/session/{channel}/buy"),
                Some(&cookie),
                r#"{"source":"t","id":"1","index":-1,"checked":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
