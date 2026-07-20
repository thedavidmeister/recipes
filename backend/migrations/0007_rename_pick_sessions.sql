-- Rename `decider_sessions` -> `pick_sessions` (#20).
--
-- The feature is `pick` — a live, shared swipe/vote — not a "cook-decider" and
-- not a separate mode. The table is internal (a pick's channel + creator +
-- optional feed filter); only its name changes. RENAME preserves the rows, the
-- primary key, and the `votes` foreign relation (votes key on channel_id, not a
-- table name).
ALTER TABLE decider_sessions RENAME TO pick_sessions;
