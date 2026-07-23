-- Equipment reading (#81): what a recipe needs you to own — pot, wok, blender, whisk.
-- A peer of ingredient_structures (#11) and step_structures (#74/#75/#76): its own
-- table, per "each enrichment its own table", not an untyped blob.
--
-- `structured` is a JSON array of RequiredEquipment, each a normalised name. The
-- normalisation is load-bearing rather than cosmetic: a kitchen selects its equipment
-- **from this vocabulary and may not invent items** (#81 ruling), because the only
-- purpose of knowing what a kitchen owns is matching it against recipes. "Frying pan"
-- and "frying Pan" as two rows would silently break every such comparison.
--
-- A reading covers preparation as well as cooking: a salad needs a bowl, a knife and a
-- board even though no heat is involved. An empty reading is therefore refused rather
-- than stored — it means the model read for appliances only, and a kitchen owning no
-- knife would otherwise appear able to cook everything.
CREATE TABLE IF NOT EXISTS equipment_structures (
    source     TEXT NOT NULL,
    id         TEXT NOT NULL,
    structured TEXT NOT NULL,
    model      TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    run_id     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source, id)
);

-- The derived view carries the reading alongside the recipe, the way `steps` does.
-- `[]` until the worker has read it — degrade-not-die: a kitchen simply has no
-- vocabulary to offer yet, rather than the page failing.
ALTER TABLE recipes ADD COLUMN equipment TEXT NOT NULL DEFAULT '[]';
