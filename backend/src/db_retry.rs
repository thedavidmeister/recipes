//! Bounded retry for transient Turso failures (#130).
//!
//! On 2026-07-25 Turso answered ~2 minutes of requests with
//! `502 Bad Gateway {"error":"upstream forward failed"}` (plus one
//! `cursor error: … unexpected EOF during chunk size line`), and every affected
//! request became a user-facing 500 — the backend ran each DB call exactly once.
//! The frontend already rides such blips out (`retryTransient` in `$lib/client`);
//! this is the backend's half: a short, bounded retry around each logical DB
//! operation, so a blip is a sub-second stall instead of an outage.
//!
//! Two pieces, deliberately separate:
//!
//! - [`is_transient`] — the **one** classifier deciding what is worth a second
//!   attempt. Everything else in the codebase asks it; nothing else inspects a
//!   database error.
//! - [`retry`] — the loop: a couple of attempts, tens-of-ms backoff, a hard cap.
//!   It takes the operation as a closure so it can be tested with injected
//!   failures and no Hrana server.
//!
//! [`crate::AppState::with_db`] composes the two with a **fresh connection per
//! attempt** — a broken Hrana stream is connection-bound (#99), so retrying on
//! the connection that just failed could replay the same dead stream forever.

use std::time::Duration;

/// How [`retry`] behaves: how many attempts in total, and the first backoff.
///
/// The delay doubles per retry, so [`POLICY`] (3 attempts, 50ms base) sleeps
/// 50ms then 100ms — at most **150ms of added latency** on top of the failing
/// attempts themselves. A truly-down Turso still fails promptly; a blip inside
/// that window turns into a stall instead of a 500.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts, the first included. Never zero.
    pub attempts: u32,
    /// Sleep before the second attempt; doubles for each retry after.
    pub base_delay: Duration,
}

/// The policy request handlers get. Small on purpose: the 2026-07-25 incident
/// errors returned in well under a second each, so two retries inside ~150ms of
/// backoff covers a per-request blip, and anything longer-lived (the full
/// ~2-minute outage) still fails fast — the client's own `retryTransient` is the
/// layer that waits out whole outages.
pub const POLICY: RetryPolicy = RetryPolicy {
    attempts: 3,
    base_delay: Duration::from_millis(50),
};

/// Is this error worth a fresh connection and another attempt?
///
/// Transient means the *transport* to Turso failed — the statement itself was
/// never judged. Anything the database actually ruled on (a constraint
/// violation, malformed SQL, a bad column) is fatal and returns immediately:
/// retrying it would burn latency to receive the same ruling again.
///
/// The typed set:
///
/// - [`libsql::Error::ConnectionFailed`] — establishing the connection failed;
///   the definition of transient.
/// - [`libsql::Error::Hrana`] — the remote protocol; mixed, see below.
/// - Everything else — fatal. This includes every SQLite-level error the local
///   path raises (`SqliteFailure`, `Sqlite3SyntaxError`, …) and the misuse
///   family (`InvalidColumnType`, `QueryReturnedNoRows`, …).
pub fn is_transient(e: &libsql::Error) -> bool {
    match e {
        libsql::Error::ConnectionFailed(_) => true,
        libsql::Error::Hrana(inner) => hrana_is_transient(&inner.to_string()),
        _ => false,
    }
}

/// Classify a Hrana error by its rendered message.
///
/// **Why a string match exists in a typed classifier**: `libsql` 0.9.30 boxes
/// its `HranaError` into `Error::Hrana(Box<dyn Error + Send + Sync>)`, and the
/// `hrana` module is private — the concrete type cannot be named, so it cannot
/// be downcast and its variants cannot be matched. The rendered message is the
/// only discriminant the crate exports. Everything over the remote protocol —
/// gateway 502s *and* genuine SQL errors — arrives through this one variant, so
/// the inspection cannot be avoided, only contained: it lives here, in this one
/// function, pinned by the tests below against the exact strings the 2026-07-25
/// incident produced. Call sites never look at message text.
///
/// The prefixes are `HranaError`'s `thiserror` templates, stable in the pinned
/// crate version (`Cargo.lock`: libsql 0.9.30). A libsql upgrade that reworks
/// them would make this classifier conservatively fatal (no retry) — the
/// pre-#130 behaviour, never a wrong retry.
fn hrana_is_transient(msg: &str) -> bool {
    // `api error: `status=…, body=…`` — the Hrana endpoint answered non-200.
    // Gateway statuses mean Turso's front door failed to reach the database;
    // the statement was never judged. Anything else (400 protocol errors, 401
    // auth) is a real answer and fatal. The incident body is matched too, in
    // case the upstream-forward failure ever rides a different status.
    if let Some(rest) = msg.strip_prefix("api error: ") {
        return rest.contains("status=502")
            || rest.contains("status=503")
            || rest.contains("status=504")
            || rest.contains("upstream forward failed");
    }
    // `http error: …` — hyper failed below HTTP: connection reset, broken
    // pipe, EOF before a response. No response, nothing was judged.
    if msg.starts_with("http error: ") {
        return true;
    }
    // `stream closed: …` — the Hrana stream died under us. Streams are
    // connection-bound; the fresh connection each attempt gets is the cure.
    if msg.starts_with("stream closed: ") {
        return true;
    }
    // `cursor error: …` — the streamed result body broke off mid-response
    // (the incident's `unexpected EOF during chunk size line`). The one
    // exception is `error at step N: …`, which is the *database* reporting a
    // statement failure through the cursor — a real ruling, fatal.
    if let Some(rest) = msg.strip_prefix("cursor error: ") {
        return !rest.contains("error at step");
    }
    // `stream error: …` — the server reported a statement-level error, which
    // is normally a real SQL ruling (constraint violation, syntax) and fatal.
    // The stream-lifecycle failures are the exception: a stream lost to a
    // restart or failover on Turso's side (#99's `stream not found:
    // generation mismatch`) is exactly what a fresh connection fixes.
    if let Some(rest) = msg.strip_prefix("stream error: ") {
        return rest.contains("stream not found") || rest.contains("generation mismatch");
    }
    // `unexpected response: …` / `json error: …` — the crate could not make
    // sense of a response it fully received. More likely a version skew than a
    // blip; fatal, so a systematic mismatch fails loudly instead of retrying.
    false
}

/// Does this error chain contain a transient database error?
///
/// The helpers the handlers call return `anyhow::Result`, which keeps the
/// concrete `libsql::Error` alive in the chain (unlike formatting it into a
/// string, which is why classification happens before any `AppError` mapping).
/// Errors with no `libsql::Error` anywhere in them are logic errors, never
/// retried.
pub fn is_transient_chain(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<libsql::Error>()
            .is_some_and(is_transient)
    })
}

/// Run `op` until it succeeds, fails fatally, or the attempts run out.
///
/// `op` receives the 1-based attempt number and is expected to acquire anything
/// connection-shaped *inside* itself, fresh each call — see
/// [`crate::AppState::with_db`]. Only errors [`is_transient_chain`] recognizes
/// are retried; after the last attempt the final error returns as-is, so the
/// existing error → 500 path downstream is unchanged.
///
/// A plain `FnMut → Future` rather than an `AsyncFnMut`, deliberately: axum
/// requires handler futures to be `Send`, and an async closure's lending future
/// cannot be bounded `Send` for every call lifetime on stable Rust — the
/// compiler rejects every handler with "implementation of `Send` is not general
/// enough". A returned-future closure keeps `Fut` independent of the closure
/// borrow, so `Send` is checked structurally and the handlers stay object-level
/// ordinary.
pub async fn retry<T, F, Fut>(policy: RetryPolicy, mut op: F) -> anyhow::Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 1;
    loop {
        match op(attempt).await {
            Ok(value) => return Ok(value),
            Err(e) if attempt < policy.attempts && is_transient_chain(&e) => {
                let delay = policy.base_delay * 2u32.pow(attempt - 1);
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    error = format!("{e:#}"),
                    "transient database error; retrying on a fresh connection"
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `Error::Hrana` carrying `msg`, exactly as the crate builds it: the
    /// variant is public and its payload is `Box<dyn Error + Send + Sync>`, so a
    /// string message reproduces what `is_transient` actually sees. This is how
    /// the wire-level shapes (which the crate offers no constructor for) get
    /// pinned.
    fn hrana(msg: &str) -> libsql::Error {
        libsql::Error::Hrana(msg.to_string().into())
    }

    /// The classifier on the messages production actually logged on 2026-07-25,
    /// verbatim — these are the errors this module exists for.
    #[test]
    fn the_incident_errors_are_transient() {
        for msg in [
            r#"api error: `status=502 Bad Gateway, body={"error":"upstream forward failed"}`"#,
            "cursor error: `unexpected EOF during chunk size line`",
        ] {
            assert!(is_transient(&hrana(msg)), "{msg:?} must be transient");
        }
    }

    #[test]
    fn gateway_statuses_and_transport_failures_are_transient() {
        for msg in [
            "api error: `status=502 Bad Gateway, body=`",
            "api error: `status=503 Service Unavailable, body=`",
            "api error: `status=504 Gateway Timeout, body=`",
            // The incident body, should it ever ride an unexpected status.
            r#"api error: `status=500 Internal Server Error, body={"error":"upstream forward failed"}`"#,
            "http error: `connection reset by peer`",
            "http error: `connection closed before message completed`",
            "stream closed: `receiving on a closed channel`",
            // Mid-stream truncation: the response body broke off.
            "cursor error: `cursor stream ended prematurely`",
            // #99's failure mode: the stream did not survive a generation
            // change on Turso's side; a fresh connection gets a new stream.
            "stream error: `Error { message: \"The stream has expired: stream not found\", code: \"STREAM_EXPIRED\" }`",
            "stream error: `Error { message: \"stream not found: generation mismatch\", code: \"INTERNAL_ERROR\" }`",
        ] {
            assert!(is_transient(&hrana(msg)), "{msg:?} must be transient");
        }
        assert!(is_transient(&libsql::Error::ConnectionFailed(
            "dns failure".into()
        )));
    }

    /// Real rulings — errors where the database judged the statement — must
    /// never be retried, whichever protocol surface delivers them.
    #[test]
    fn database_rulings_are_fatal() {
        for msg in [
            // A remote constraint violation or syntax error arrives as a
            // stream-level statement error with an SQLITE code.
            "stream error: `Error { message: \"SQLite error: UNIQUE constraint failed: users.telegram_user_id\", code: \"SQLITE_CONSTRAINT\" }`",
            "stream error: `Error { message: \"SQL string could not be parsed\", code: \"SQL_PARSE_ERROR\" }`",
            // …or, mid-cursor, as a step error.
            "cursor error: `error at step 0: SQLite error: NOT NULL constraint failed: recipes.title`",
            // A non-gateway API answer is a real answer: 400s are protocol
            // errors, 401 says the auth token is wrong. Retrying a bad token
            // would hammer Turso with the same bad token.
            r#"api error: `status=400 Bad Request, body={"error":"invalid baton"}`"#,
            r#"api error: `status=401 Unauthorized, body=`"#,
            // A fully-received but unintelligible response is skew, not a blip.
            "unexpected response: `HttpBody(...)`",
            "json error: `expected value at line 1 column 1`",
        ] {
            assert!(!is_transient(&hrana(msg)), "{msg:?} must be fatal");
        }
    }

    /// The fatal classes as *real* typed errors, provoked against a real
    /// database rather than constructed — the local path's equivalents of what
    /// the remote wraps in `Hrana`.
    #[tokio::test]
    async fn real_constraint_and_syntax_errors_are_fatal() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", ())
            .await
            .unwrap();
        conn.execute("INSERT INTO t (id) VALUES (1)", ())
            .await
            .unwrap();

        let constraint = conn
            .execute("INSERT INTO t (id) VALUES (1)", ())
            .await
            .unwrap_err();
        assert!(
            !is_transient(&constraint),
            "a constraint violation must not be retried: {constraint}"
        );

        let syntax = conn.execute("NOT EVEN SQL", ()).await.unwrap_err();
        assert!(
            !is_transient(&syntax),
            "malformed SQL must not be retried: {syntax}"
        );
    }

    /// A transient error wrapped in `anyhow` context — the shape the helpers
    /// actually produce — still classifies through the chain.
    #[test]
    fn classification_survives_anyhow_context() {
        use anyhow::Context as _;

        let wrapped: anyhow::Error = Err::<(), _>(hrana("http error: `connection reset by peer`"))
            .context("could not load the corpus")
            .unwrap_err();
        assert!(is_transient_chain(&wrapped));

        let logic = anyhow::anyhow!("a plain logic error");
        assert!(!is_transient_chain(&logic));
    }

    fn transient_err() -> anyhow::Error {
        hrana(r#"api error: `status=502 Bad Gateway, body={"error":"upstream forward failed"}`"#)
            .into()
    }

    #[tokio::test]
    async fn a_blip_is_ridden_out() {
        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        let result = retry(POLICY, move |_| async move {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(transient_err())
            } else {
                Ok("recovered")
            }
        })
        .await;
        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(calls.get(), 3, "two transient failures, then success");
    }

    /// A fatal error gets exactly one attempt — no retry, no sleep.
    #[tokio::test]
    async fn a_fatal_error_fails_at_once() {
        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        let result: anyhow::Result<()> = retry(POLICY, move |_| async move {
            calls.set(calls.get() + 1);
            Err(anyhow::Error::from(hrana(
                "stream error: `Error { message: \"SQLite error: UNIQUE constraint failed\", code: \"SQLITE_CONSTRAINT\" }`",
            )))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 1, "a database ruling must not be retried");
    }

    /// An error with no `libsql::Error` in it is a logic error, not a blip.
    #[tokio::test]
    async fn a_non_database_error_fails_at_once() {
        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        let result: anyhow::Result<()> = retry(POLICY, move |_| async move {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("boom"))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
    }

    /// A dead database costs the bounded attempts and then the *last* error —
    /// never a hang, never an extra attempt.
    #[tokio::test]
    async fn a_dead_database_fails_after_bounded_attempts() {
        let calls = std::cell::Cell::new(0u32);
        let calls = &calls;
        let result: anyhow::Result<()> = retry(POLICY, move |attempt| async move {
            calls.set(calls.get() + 1);
            assert_eq!(attempt, calls.get(), "op sees the 1-based attempt number");
            Err(transient_err())
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), POLICY.attempts, "bounded by the policy");
    }
}
