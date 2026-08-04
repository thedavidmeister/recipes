-- The pre-existing session events move onto the event framework (#209), and each one
-- gains the fact the framework exists to carry: **when it happened**.
--
-- #208 built `backend/src/events.rs` — one place a session event is timed, authorised,
-- applied and announced — with cook timers as its first consumer. This migration is the
-- storage half of the second consumer wave: votes, shopping ticks, the pantry seed and
-- the decision now arrive through that choke point, so for the first time each of them
-- knows the instant the initiator raised it rather than only the moment a row happened
-- to be written.
--
-- ## Additive, and nullable, and nothing is backfilled
--
-- Four `ADD COLUMN`s and not one rewrite. Every existing column keeps the meaning it
-- has today: `votes.created_at`, `buy_checks.created_at` and `buy_seeds.seeded_at` are
-- still the row's own `unixepoch()` write time, and `pick_sessions.decided_at` is still
-- both *when* the plan decided and — as the column `WHERE decided_at IS NULL` is written
-- against — *whether* it has. Nothing here touches that guard, and nothing here is a
-- second place the decision could be read from.
--
-- The new columns are **nullable with no default**, which is 0026's ruling applied
-- again: a row written before this deployed has no honest initiator instant to state,
-- and `unixepoch() * 1000` would be a guess dressed as a measurement — the receipt, not
-- the tap, which is precisely the mistake the framework exists to refuse. NULL reads as
-- "this predates the framework and we do not know", and every surface that would show
-- one has to say so rather than round a fabrication to the second.
--
-- ## Milliseconds, on the shared timeline
--
-- Unix **milliseconds**, like `plan_timers.started_at_ms` (0027) and unlike the second
-- resolution the session tables use, and on the **shared timeline** — the initiator's own
-- clock at the moment of the action, corrected for that participant's measured drift by
-- `events::normalize`. Two facts follow and both matter:
--
-- * these are not comparable to the `unixepoch()` columns beside them by a factor of a
--   thousand, which is why they are separate columns with separate names rather than a
--   widening of the old ones; and
-- * they are not the server's receipt. A phone that stalls in a tunnel between the tap
--   and the frame arriving still records the tap, and a phone forty minutes fast records
--   the same real instant as everybody else's.
--
-- ## Numbering: why 28
--
-- `db.rs` applies by `MAX(version)`, so a number at or below one already applied on
-- production never runs. 0027 (#208) is the highest in the tree and is on `main`, so 28
-- is the next number above everything.

-- A swipe's own instant (#175/#201 own the row; this is only *when* it was cast).
-- Re-voting overwrites the row, and this column moves with it: a swipe is a person's
-- current call, so the instant recorded is the instant of the call that stands.
ALTER TABLE votes ADD COLUMN created_at_ms INTEGER;

-- A shopping claim's own instant (#131/#156). A tick is a take-over — the key does not
-- include the person — so this moves to whoever holds the line now, the same way
-- `user_id` and `created_at` already do.
--
-- A **pantry pre-tick** (#156) carries one too, and it is honestly the server's: the
-- kitchen did not tap anything, the seed ran. See `session::write_seed`.
ALTER TABLE buy_checks ADD COLUMN created_at_ms INTEGER;

-- When the seed ran, in the shared timeline. Beside `seeded_at` rather than instead of
-- it: `seeded_at` is what 0021 promised and what the one-time marker is read by, and
-- this is the instant at the framework's resolution.
ALTER TABLE buy_seeds ADD COLUMN seeded_at_ms INTEGER;

-- **When the plan decided** — the deciding swipe's own instant, not the moment the
-- UPDATE ran.
--
-- The decision is the one event here nobody raises directly: it is a consequence of the
-- last yes, evaluated inside that vote's own write (#205), so the instant it happened at
-- *is* that vote's instant and there is nothing else it could honestly be. A plan whose
-- deciding tap was made on a phone that then spent ninety seconds reconnecting decided
-- when the tap happened.
--
-- `decided_at` is untouched, still `unixepoch()`, and still the column the win
-- condition's `decided_at IS NULL` predicate is written against. This one says nothing
-- about *whether*: it is NULL on every plan decided before this deployed, and reading it
-- as a decision flag would call those plans undecided.
ALTER TABLE pick_sessions ADD COLUMN decided_at_ms INTEGER;
