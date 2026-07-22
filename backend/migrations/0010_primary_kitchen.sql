-- Every person has a primary kitchen (#72): the one the app assumes unless they have
-- explicitly switched to another. It exists so there is no such thing as having no
-- kitchen — nothing to create before you can cook, and no empty state to design for.
--
-- It is a property of a *membership*, not of a kitchen. Your primary is yours alone: a
-- kitchen you own is not automatically anybody else's primary, and a guest you invite
-- keeps whatever primary they already had.
ALTER TABLE kitchen_members ADD COLUMN is_primary INTEGER NOT NULL DEFAULT 0;

-- At most one primary per person. This is what makes "make sure one exists" safe to
-- run from an ordinary request: two first visits racing each other resolve here, and
-- the loser is refused by the index rather than seating a second primary.
CREATE UNIQUE INDEX IF NOT EXISTS kitchen_members_primary
    ON kitchen_members (user_id) WHERE is_primary = 1;
