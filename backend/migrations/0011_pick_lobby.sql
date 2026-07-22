-- A meal plan starts in a lobby (#20, #72): the people who will decide gather first,
-- and only then does the swiping begin.
--
-- The roster this creates is the answer to "who has to agree". Before it, that had to
-- be inferred — from who had voted so far, which reads as one person until a friend
-- swipes, or from who happens to be connected, which a reload makes a lie. An
-- explicit list is neither: you are in the plan because you joined it, and you stay in
-- it through a dropped connection or a walk to the kitchen.
--
-- A plan belongs to a kitchen, which is what will let it be scoped to that kitchen's
-- pantry and equipment. `started_at` is NULL for as long as the lobby is open; setting
-- it is what closes the roster and starts the pick.
ALTER TABLE pick_sessions ADD COLUMN kitchen_id TEXT;
ALTER TABLE pick_sessions ADD COLUMN started_at INTEGER;

-- Who is deciding. One row per person per plan; the creator is seated on creation, so
-- a plan is never roster-less.
CREATE TABLE IF NOT EXISTS pick_voters (
    channel_id TEXT NOT NULL,
    user_id    TEXT NOT NULL,   -- telegram_user_id, denormalized as everywhere else
    joined_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (channel_id, user_id)
);
