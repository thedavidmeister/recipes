-- Kitchens (#72): the shared space — an owner and invited guests — that the meal
-- flow (pick · buy · cook · joy, #36) runs inside, with an equipment list and a
-- pantry. Identity is the Telegram id everywhere (#25), stored denormalized as a
-- string (the pick_sessions convention), not an FK to users.
CREATE TABLE IF NOT EXISTS kitchens (
    id           TEXT NOT NULL PRIMARY KEY,   -- opaque, minted server-side
    name         TEXT NOT NULL,
    owner_id     TEXT NOT NULL,               -- the telegram_user_id that created it
    invite_token TEXT NOT NULL,               -- shareable; redeeming it joins as a guest
    created_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
-- One kitchen per invite token — the join looks a kitchen up by it.
CREATE UNIQUE INDEX IF NOT EXISTS kitchens_invite_token ON kitchens (invite_token);

-- Membership: the owner and every guest, one row each. Role is 'owner' | 'guest'.
CREATE TABLE IF NOT EXISTS kitchen_members (
    kitchen_id TEXT NOT NULL,
    user_id    TEXT NOT NULL,   -- telegram_user_id
    role       TEXT NOT NULL,
    joined_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (kitchen_id, user_id)
);
-- "which kitchens am I in" is the list query — index the member side.
CREATE INDEX IF NOT EXISTS kitchen_members_by_user ON kitchen_members (user_id);

-- What the kitchen has: appliances/tools (equipment) and stock on hand (pantry).
-- Two tables, same shape — one item per row, keyed by (kitchen, item) so an add is
-- an idempotent upsert. Kept as free text for now (#72 open question: a controlled
-- vocabulary vs the corpus normalizer).
CREATE TABLE IF NOT EXISTS kitchen_equipment (
    kitchen_id TEXT NOT NULL,
    item       TEXT NOT NULL,
    added_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (kitchen_id, item)
);

CREATE TABLE IF NOT EXISTS kitchen_pantry (
    kitchen_id TEXT NOT NULL,
    item       TEXT NOT NULL,
    added_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (kitchen_id, item)
);
