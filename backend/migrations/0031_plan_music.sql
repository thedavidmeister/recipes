-- Everyone in a plan hears the same music (#212): the session owns the soundtrack.
--
-- The music was a per-device dice roll. Each browser's layout picked its own random
-- track from the section's pool and started it whenever that device happened to arrive,
-- so two people shopping the same list — or standing at the same stove — heard
-- different songs at different points. It is presented as the *meal's* atmosphere and
-- it was actually each phone's.
--
-- ## Two stored facts, and the position derived from them
--
-- `track` is **what the room is playing**, chosen server-side (`music::choose`) so the
-- no-back-to-back-repeat rule is one shuffle for the room instead of one per phone. It
-- is never taken from the wire: a track name from a client is a URL every phone in the
-- plan would then load.
--
-- `started_at_ms` is **when it started**, in the shared timeline — the instant the
-- previous track ended on the device that reported it, normalised through that
-- participant's own measured clock drift by `events::normalize` (the app's time-sync
-- framework, `backend/src/events.rs`). It is not the server's receipt: a phone that
-- stalls between the track ending and the frame arriving must not shift the room.
--
-- **Playback position is derived, never stored** (`now − started_at_ms`, read through
-- each device's recorded offset) — the rule 0027 states for a timer's deadline and #162
-- states for per-serving calories. A stored position would need a writer at every
-- instant, would be wrong the moment nothing was connected to update it, and would be a
-- third number free to disagree with the two it came from.
--
-- Milliseconds, like `plan_timers.started_at_ms` (0027) and unlike the `unixepoch()`
-- seconds the older session tables use: a second of quantisation at the anchor is a
-- second of disagreement between two speakers in one kitchen, which is audible.
--
-- ## One row per section, and the row is the whole state
--
-- Keyed by `(channel_id, section)` — the four legs of a meal (`pick`, `buy`, `cook`,
-- `joy`), which is what a room moves through together. No history: a rollover is an
-- UPDATE of the one row, because what a plan is playing *now* is the only thing any
-- device needs and a log of what it played would be a table nobody reads.
--
-- **`started_at_ms` is also the compare-and-set token**, which is what makes several
-- devices reporting the same rollover come to one answer: the update matches only while
-- the instant it names is still the current one, so exactly one call changes the row and
-- the rest are told what it chose (#205's `decided_at IS NULL` discipline, generalised
-- from "first past the post" to "answering the state that is actually current").
--
-- `kitchens` has music and is **not** a section here: it has no plan behind it, so
-- there is no room to share it with and the device-local path serves it.
--
-- ## Numbering: why 31
--
-- `db.rs` applies by `MAX(version)`, so a number at or below one already applied on
-- production never runs at all. 0028 (#209) is the highest on `main`, and 29 and 30 are
-- claimed by work in flight beside this branch, so 31 is the next number above
-- everything — the only choice that is safe whichever branch deploys first.
CREATE TABLE IF NOT EXISTS plan_music (
    channel_id    TEXT NOT NULL,    -- the meal session (pick_sessions.channel_id)
    section       TEXT NOT NULL,    -- pick | buy | cook | joy
    track         TEXT NOT NULL,    -- the room's current track, chosen server-side
    started_at_ms INTEGER NOT NULL, -- when it began, in the shared timeline

    -- No `user_id`: a soundtrack is nobody's claim. A timer and a shopping line record
    -- whose hand is on them because a person took them; the room's music is the room's,
    -- and whoever's phone happened to notice a track end is not an owner of it.
    PRIMARY KEY (channel_id, section)
);

-- No secondary index: music is read by (channel_id, section) or by channel_id alone —
-- the whole key and a prefix of it, both served by SQLite's automatic index on it.
