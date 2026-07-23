-- Everyone in a kitchen is an owner (#72).
--
-- A kitchen is a place people share, not a thing one person holds and lends out.
-- Whoever is in it can change it, invite to it, and stock it — so a guest role was
-- describing a difference that does not exist, and every read had to carry a value
-- that only ever meant one thing.
--
-- The column goes rather than becoming a constant. A field that always holds the same
-- answer is a question the schema keeps asking and the code keeps pretending to weigh.
UPDATE kitchen_members SET role = 'owner';
ALTER TABLE kitchen_members DROP COLUMN role;
