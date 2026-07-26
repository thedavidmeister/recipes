//! Boot that degrades instead of dying (#146).
//!
//! On 2026-07-25 Turso's `aws-eu-west-1` edge truncated every `/v3/cursor`
//! response for about seven hours. Requests failing was expected — [`crate::db_retry`]
//! softens that. The compounding failure was **boot**: migrations ran inline and
//! a failure exited the process, so four consecutive deploys died with
//! `Error: Hrana: cursor error … unexpected EOF` → `Exited with status 1`, the
//! last image that ever booted stayed live serving 500s, and nothing could ship
//! until a human redeployed after the provider recovered — including the very PR
//! that hardened the request path. The backend is on Render's free tier and
//! spins down at 15 minutes idle, so **every cold start is a boot**: a blip that
//! coincides with a wake-up is a hard outage rather than a slow request.
//!
//! So the process binds its port and serves whatever the database is doing, and
//! the schema comes up **beside** the server rather than in front of it:
//!
//! - [`Readiness`] is the one shared flag. [`crate::AppState`] carries it, the
//!   router's schema gate reads it, and `/api/health` reports it.
//! - [`migrate_until_ready`] is the boot task: attempt, and on a *transient*
//!   failure ([`crate::db_retry::is_transient_chain`]) wait and attempt again,
//!   for as long as it takes. It takes the attempt as a closure, so the whole
//!   schedule is testable with injected failures and no Hrana server.
//!
//! ## What still refuses to boot, and what only degrades
//!
//! The line is **whether the network was involved in the verdict**:
//!
//! - **Decided from config alone → exit, before the port is bound.** A missing
//!   or unrecognized `DATABASE_URL`, a missing `TURSO_AUTH_TOKEN`, missing
//!   Telegram secrets. [`crate::db::open`] resolves these without sending a
//!   packet (`Builder::new_remote(…).build()` allocates a handle; it does not
//!   connect), so the verdict cannot be a provider having a bad hour. The
//!   process could never serve anything, and a placeholder `DATABASE_URL` that
//!   boots "successfully" against nothing is the failure `db.rs` was written to
//!   refuse.
//! - **Decided by talking to the database → never fatal to the process.** That
//!   includes a fatal *ruling* (bad credentials, a migration the database
//!   rejects): the schema stays [`Schema::Failed`] permanently, every
//!   schema-dependent route answers 503, `/api/health` says so, and the error is
//!   logged at `error!`. Exiting here would re-arm the deploy freeze this module
//!   exists to end — the transient/fatal split is a heuristic over strings the
//!   *provider* controls (see [`crate::db_retry`]), and one misfiled provider
//!   error would kill every deploy again. A dead container is also the quietest
//!   possible signal: `/api/health` answering `"database":"failed"` is louder,
//!   because it can still be asked.
//!
//! The case exiting would protect — a broken migration shipping and taking the
//! site down — is already gated earlier: `db::migrate` applies **every**
//! migration in the test suite, so SQL the database rejects cannot reach a
//! deploy.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// What this process currently knows about the schema.
///
/// The three states are what a prober and an operator actually need to tell
/// apart: still coming up, serving, or stuck until someone acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    /// Migrations have not completed. The boot task is still attempting them, so
    /// this state resolves itself when the database comes back.
    Pending,
    /// Migrations are applied. Every route can serve.
    Ready,
    /// The database *ruled* on a migration attempt — a credential it refused, SQL
    /// it rejected. Retrying would collect the same ruling, so the boot task has
    /// stopped and this needs a human.
    Failed,
}

impl Schema {
    /// The word `/api/health` reports.
    ///
    /// Part of the endpoint's contract: a prober matches on it, so these strings
    /// are as much API as the JSON keys around them.
    pub fn as_str(self) -> &'static str {
        match self {
            Schema::Pending => "pending",
            Schema::Ready => "ready",
            Schema::Failed => "failed",
        }
    }

    /// Can schema-dependent routes serve?
    pub fn is_ready(self) -> bool {
        self == Schema::Ready
    }

    fn from_u8(v: u8) -> Schema {
        match v {
            READY => Schema::Ready,
            FAILED => Schema::Failed,
            _ => Schema::Pending,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Schema::Pending => PENDING,
            Schema::Ready => READY,
            Schema::Failed => FAILED,
        }
    }
}

const PENDING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

/// The shared "can we serve?" flag, written by the boot task and read by every
/// request.
///
/// An atomic rather than a lock: it is read on the hot path of every
/// schema-dependent request and written a handful of times in a process's life,
/// so there is nothing to guard and no reason for a request to be able to block
/// behind the writer.
#[derive(Debug, Clone)]
pub struct Readiness(Arc<AtomicU8>);

impl Readiness {
    /// A process that has not migrated yet — how every boot starts.
    pub fn pending() -> Readiness {
        Readiness(Arc::new(AtomicU8::new(PENDING)))
    }

    /// A schema known to be current, for the CLI paths and tests that migrate
    /// before they build state.
    pub fn ready() -> Readiness {
        Readiness(Arc::new(AtomicU8::new(READY)))
    }

    pub fn get(&self) -> Schema {
        Schema::from_u8(self.0.load(Ordering::Acquire))
    }

    pub fn set(&self, schema: Schema) {
        self.0.store(schema.as_u8(), Ordering::Release);
    }
}

/// How long [`migrate_until_ready`] waits between attempts.
///
/// Exponential from `base`, doubling per failed attempt, clamped at `cap`. There
/// is deliberately **no attempt limit** — the delay is the bound. A limit would
/// mean an outage longer than the limit leaves a process that never recovers
/// even after the provider does, which is the shape of the original bug.
#[derive(Debug, Clone, Copy)]
pub struct Schedule {
    /// Wait before the second attempt.
    pub base: Duration,
    /// The longest this ever waits, however long the outage runs.
    pub cap: Duration,
}

/// The schedule a deploy boots with.
///
/// 1s, 2s, 4s … 60s, then 60s forever: a blip of a few seconds costs a few
/// seconds of 503s, and the seven-hour incident this module is named for would
/// have cost ~430 attempts — about one a minute, which is less traffic than a
/// single browser tab polling. Fast enough that a normal wake-up is invisible,
/// slow enough that a whole bad afternoon is not a load test aimed at a provider
/// that is already struggling.
pub const SCHEDULE: Schedule = Schedule {
    base: Duration::from_secs(1),
    cap: Duration::from_secs(60),
};

/// How long to wait after `attempt` (1-based) failed.
///
/// Saturating throughout: a process that has been retrying for days must not
/// panic on an overflowed shift or a Duration that ran off the end of `u64`.
fn backoff(schedule: Schedule, attempt: u32) -> Duration {
    // Clamped to 31 so the shift is always defined for `u32`; anything past the
    // first handful of doublings is already at `cap` anyway.
    let doublings = attempt.saturating_sub(1).min(31);
    schedule
        .base
        .saturating_mul(1u32 << doublings)
        .min(schedule.cap)
}

/// Bring the schema up, however long it takes, and publish the outcome.
///
/// `attempt` receives the 1-based attempt number and is expected to acquire its
/// own connection *inside* itself, fresh each call — the same rule
/// [`crate::AppState::with_db`] follows, and for the same reason: a libsql
/// connection owns a Hrana stream, and the stream that just broke is exactly
/// what must not be reused (#99). Taking the work as a closure is also the test
/// seam — the schedule and the fatal/transient split are exercised with injected
/// failures rather than a fake Turso.
///
/// Returns when the schema is [`Schema::Ready`] or [`Schema::Failed`]; the
/// caller spawns it and never awaits it, because the whole point is that serving
/// does not wait for this.
pub async fn migrate_until_ready<F, Fut>(schedule: Schedule, readiness: Readiness, mut attempt: F)
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let mut n: u32 = 1;
    loop {
        match attempt(n).await {
            Ok(()) => {
                readiness.set(Schema::Ready);
                tracing::info!(attempts = n, "database schema is ready");
                return;
            }
            // The transport to Turso failed, so the migration was never judged.
            // Wait and ask again — this is the seven-hour-outage case, and the
            // service keeps answering 503 throughout.
            Err(e) if crate::db_retry::is_transient_chain(&e) => {
                let delay = backoff(schedule, n);
                tracing::warn!(
                    attempt = n,
                    delay_ms = delay.as_millis() as u64,
                    error = format!("{e:#}"),
                    "database unreachable; the service is serving 503 on schema routes and will retry migrations"
                );
                tokio::time::sleep(delay).await;
                n = n.saturating_add(1);
            }
            // The database answered, and its answer was no. Retrying collects the
            // same no. Fail loudly and stay up: `/api/health` reporting `failed`
            // is a signal a human can query, which an exited container is not.
            Err(e) => {
                readiness.set(Schema::Failed);
                tracing::error!(
                    attempt = n,
                    error = format!("{e:#}"),
                    "migrations failed on a database ruling, not a blip — the schema will not come up without a fix; \
                     every schema-dependent route now answers 503 and /api/health reports \"database\":\"failed\""
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A transient `libsql` error, verbatim from the 2026-07-25 boot logs — the
    /// one that killed four deploys.
    fn boot_incident_error() -> anyhow::Error {
        libsql::Error::Hrana(
            "cursor error: `error reading a body from connection: unexpected EOF during chunk size line`"
                .to_string()
                .into(),
        )
        .into()
    }

    /// A ruling: the database judged the statement and said no.
    fn ruling() -> anyhow::Error {
        libsql::Error::Hrana(
            "stream error: `Error { message: \"SQLite error: near \\\"NOT\\\": syntax error\", code: \"SQL_PARSE_ERROR\" }`"
                .to_string()
                .into(),
        )
        .into()
    }

    /// Attempts land 1ms apart, so a test can exercise hundreds of them.
    const FAST: Schedule = Schedule {
        base: Duration::from_millis(1),
        cap: Duration::from_millis(1),
    };

    #[test]
    fn the_backoff_doubles_then_holds_at_the_cap() {
        assert_eq!(backoff(SCHEDULE, 1), Duration::from_secs(1));
        assert_eq!(backoff(SCHEDULE, 2), Duration::from_secs(2));
        assert_eq!(backoff(SCHEDULE, 3), Duration::from_secs(4));
        assert_eq!(backoff(SCHEDULE, 7), Duration::from_secs(60), "clamped");
        assert_eq!(backoff(SCHEDULE, 8), Duration::from_secs(60));

        // A process that has been retrying for days keeps retrying: no shift
        // overflow, no Duration overflow, never a zero delay that would spin.
        for attempt in [32u32, 33, 1_000, u32::MAX] {
            assert_eq!(
                backoff(SCHEDULE, attempt),
                SCHEDULE.cap,
                "attempt {attempt} must sit at the cap"
            );
        }
        // Roughly the seven-hour incident, at one attempt per capped minute.
        let hours_at_cap = 7 * 60 * 60 / SCHEDULE.cap.as_secs();
        assert!(
            (60..=1_000).contains(&hours_at_cap),
            "a seven-hour outage should cost hundreds of attempts, not tens of thousands: {hours_at_cap}"
        );
    }

    /// The headline (#146): a transient failure at boot does **not** end the
    /// process, and the schema flips to ready the moment the provider comes back.
    #[tokio::test]
    async fn a_transient_failure_keeps_retrying_and_then_readies() {
        let readiness = Readiness::pending();
        let calls = Cell::new(0u32);
        let calls = &calls;
        // A borrow, so the `FnMut` closure can look at the flag on every attempt
        // — the same flag the request path reads.
        let observed = &readiness;
        migrate_until_ready(FAST, readiness.clone(), move |n| async move {
            calls.set(calls.get() + 1);
            assert_eq!(n, calls.get(), "the attempt number is 1-based");
            if n < 4 {
                assert_eq!(
                    observed.get(),
                    Schema::Pending,
                    "the service reports pending for the whole window"
                );
                Err(boot_incident_error())
            } else {
                Ok(())
            }
        })
        .await;
        assert_eq!(calls.get(), 4);
        assert_eq!(readiness.get(), Schema::Ready, "and it recovers by itself");
    }

    /// There is no attempt limit — the cap on the *delay* is the only bound, so
    /// an outage longer than any limit we might have picked still recovers.
    #[tokio::test]
    async fn there_is_no_attempt_limit() {
        let readiness = Readiness::pending();
        let calls = Cell::new(0u32);
        let calls = &calls;
        migrate_until_ready(FAST, readiness.clone(), move |_| async move {
            calls.set(calls.get() + 1);
            if calls.get() < 250 {
                Err(boot_incident_error())
            } else {
                Ok(())
            }
        })
        .await;
        assert_eq!(readiness.get(), Schema::Ready);
        assert_eq!(calls.get(), 250);
    }

    /// A ruling is not a blip: one attempt, then a permanent, loud not-ready.
    /// Never an exit — the process keeps answering `/api/health` so the failure
    /// is something an operator can ask about.
    #[tokio::test]
    async fn a_database_ruling_stops_at_once_and_stays_failed() {
        let readiness = Readiness::pending();
        let calls = Cell::new(0u32);
        let calls = &calls;
        migrate_until_ready(FAST, readiness.clone(), move |_| async move {
            calls.set(calls.get() + 1);
            Err(ruling())
        })
        .await;
        assert_eq!(calls.get(), 1, "a ruling must not be retried");
        assert_eq!(readiness.get(), Schema::Failed);
        assert!(!readiness.get().is_ready());
    }

    /// A failure with no `libsql::Error` in it — a logic error in the boot path
    /// itself — is a ruling too, not something to hammer the database over.
    #[tokio::test]
    async fn a_non_database_failure_is_fatal_to_the_schema_not_the_process() {
        let readiness = Readiness::pending();
        migrate_until_ready(FAST, readiness.clone(), |_| async {
            Err(anyhow::anyhow!("boom"))
        })
        .await;
        assert_eq!(readiness.get(), Schema::Failed);
    }

    /// A healthy database: one attempt, ready, nothing waited on.
    #[tokio::test]
    async fn a_healthy_database_readies_on_the_first_attempt() {
        let readiness = Readiness::pending();
        let calls = Cell::new(0u32);
        let calls = &calls;
        migrate_until_ready(SCHEDULE, readiness.clone(), move |_| async move {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .await;
        assert_eq!(calls.get(), 1);
        assert_eq!(readiness.get(), Schema::Ready);
    }

    /// The words `/api/health` publishes, and the flag every request reads.
    #[test]
    fn the_reported_states_are_distinguishable() {
        assert_eq!(Schema::Pending.as_str(), "pending");
        assert_eq!(Schema::Ready.as_str(), "ready");
        assert_eq!(Schema::Failed.as_str(), "failed");
        assert!(Schema::Ready.is_ready());
        assert!(!Schema::Pending.is_ready());
        assert!(!Schema::Failed.is_ready());

        // Every state survives the atomic round trip a shared flag needs.
        for schema in [Schema::Pending, Schema::Ready, Schema::Failed] {
            let readiness = Readiness::pending();
            readiness.set(schema);
            assert_eq!(readiness.get(), schema);
            // …and a clone is the *same* flag, which is what makes the boot task
            // and the request path agree.
            assert_eq!(readiness.clone().get(), schema);
        }
        assert_eq!(Readiness::ready().get(), Schema::Ready);
    }
}
