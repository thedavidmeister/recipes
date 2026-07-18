//! Enrich: read a recipe's raw ingredient lines into structure (#11), stored
//! per recipe.
//!
//! This is the one **networked** stage of the corpus pipeline and the only writer
//! of `ingredient_structures`. For each recipe that has no structured reading yet,
//! it sends that recipe's ingredient lines to an LLM in one call and stores the
//! readings — one row per recipe (`source, id`), the array aligned to the recipe's
//! ingredient order. It does **not** write `recipes` — [`crate::derive`] reattaches
//! from this table, which is why deriving stays offline.
//!
//! **Per recipe, not per line.** A reading is captured once per recipe (matching
//! #11's "once per recipe at write time"), which keeps the raw → enrich → derive
//! chain a clean per-`(source, id)` cascade with no line→recipe fan-out, and lets
//! this be a dedicated table rather than a generic `(kind, json)` container. A
//! future enrichment (nutrition, allergens) is its own table + extractor, not a
//! row here.
//!
//! **A capture, not a derivation.** `recipes` is a deterministic derivation of
//! `raw_imports`; a reading is not — a model is non-deterministic and drifts, so a
//! reading is a point-in-time artifact, a peer of `raw_imports`. We keep the
//! capture rather than re-roll a drifting model on every derive; each row records
//! its provenance ([`Extractor::provenance`] + a timestamp) so drift is auditable,
//! and a deliberate re-snapshot with a better model is an explicit act (`enrich
//! --refresh`), never a silent side effect.
//!
//! The LLM boundary is a trait ([`Extractor`]) so the engine runs against a
//! fixture with no network, the same shape [`crate::sync`] uses — and so the
//! **provider is not baked in**. Reading a line into JSON is a commodity task, so
//! production is [`OpenAiCompatExtractor`]: one call to any OpenAI-compatible
//! `/chat/completions` endpoint (OpenAI, OpenRouter, Together, Groq, a local
//! Ollama/vLLM), picked per deployment by env, constrained to the
//! [`StructuredMeasure`] schema by structured output.
//!
//! **Degrade-not-die.** With no endpoint configured there is no extractor, enrich
//! is a no-op, and derive leaves recipes' `structured` fields `None`. The corpus
//! still ingests and serves — enrichment is an addition, never a gate.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use libsql::Connection;
use recipe_core::{Ingredient, StructuredMeasure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A recipe key: `(source, id)`.
type RecipeKey = (String, String);

/// Reads one recipe's ingredient lines into one [`StructuredMeasure`] each.
///
/// Production is an LLM call ([`OpenAiCompatExtractor`]); tests use a fixture. The
/// engine is generic over this so it runs with no network, and so no single LLM
/// vendor is wired into the corpus.
pub trait Extractor {
    /// Read `lines` into one reading each, **in the same order**. Must return
    /// exactly `lines.len()` readings or an error — the engine treats a count
    /// mismatch as a failed extraction rather than misaligning readings onto the
    /// wrong lines.
    fn extract(
        &self,
        lines: &[Ingredient],
    ) -> impl Future<Output = anyhow::Result<Vec<StructuredMeasure>>>;

    /// A label for what produced these readings — the model id — stored with each
    /// recipe's row. Provenance for a non-deterministic, versioned source: it makes
    /// "which model read this recipe, and roughly when" answerable, and a targeted
    /// re-capture possible.
    fn provenance(&self) -> String;
}

/// What an enrich run did — counted in **recipes**.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct EnrichReport {
    /// Recipes that needed a reading this run (had ingredients, not yet stored).
    pub missing: usize,
    /// Recipes newly read and written to `ingredient_structures`.
    pub enriched: usize,
    /// Recipes whose extraction failed — left unstored, so the recipe stays
    /// unenriched until a later run succeeds. A failure is never fatal.
    pub failed: usize,
}

/// Read every recipe that has no structured reading yet into `ingredient_structures`.
///
/// One [`Extractor`] call per recipe (its whole ingredient list), one row written
/// per recipe. A recipe already stored is skipped — not because a reading is a pure
/// function of its lines (it is not; the model drifts), but because we keep the
/// captured reading rather than re-roll on every run. `refresh = true` is the
/// deliberate re-capture: re-read every recipe and overwrite with the current
/// model, kept out of the routine path so a model change never silently re-pays for
/// the whole corpus.
pub async fn enrich<E: Extractor>(
    conn: &Connection,
    extractor: &E,
    refresh: bool,
) -> anyhow::Result<EnrichReport> {
    let mut report = EnrichReport::default();
    let provenance = extractor.provenance();
    // Recipes we can skip: on a routine run, everything already stored; on a
    // refresh, none — every recipe is re-read.
    let done: HashSet<RecipeKey> = if refresh {
        HashSet::new()
    } else {
        stored_recipes(conn).await?
    };

    for (source, id, ingredients) in recipe_ingredients(conn).await? {
        if ingredients.is_empty() || done.contains(&(source.clone(), id.clone())) {
            continue;
        }
        report.missing += 1;

        match extractor.extract(&ingredients).await {
            Ok(readings) if readings.len() == ingredients.len() => {
                store(conn, &source, &id, &readings, &provenance).await?;
                report.enriched += 1;
            }
            Ok(readings) => {
                tracing::warn!(
                    "extractor returned {} readings for {}/{} ({} lines) — skipping",
                    readings.len(),
                    source,
                    id,
                    ingredients.len()
                );
                report.failed += 1;
            }
            Err(e) => {
                tracing::warn!("extraction failed for {source}/{id}: {e}");
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Load every recipe's readings into a map so [`crate::derive`] can reattach in
/// memory — one query, not a lookup per recipe.
pub async fn load(conn: &Connection) -> anyhow::Result<HashMap<RecipeKey, Vec<StructuredMeasure>>> {
    let mut rows = conn
        .query(
            "SELECT source, id, structured FROM ingredient_structures",
            (),
        )
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let structured: String = row.get(2)?;
        // A row that no longer deserializes (a shape change) is skipped, not fatal.
        if let Ok(readings) = serde_json::from_str::<Vec<StructuredMeasure>>(&structured) {
            map.insert((source, id), readings);
        }
    }
    Ok(map)
}

/// Reattach a recipe's readings onto its ingredients in place — the join `derive`
/// performs, offline. Attaches only when the stored array still lines up with the
/// recipe's ingredients (same count): a reading left over from a since-changed raw
/// simply doesn't attach (the recipe re-enriches next run) rather than misaligning.
/// A recipe with no row keeps `structured: None` — raw stays the source of truth.
pub fn attach(
    readings_by_recipe: &HashMap<RecipeKey, Vec<StructuredMeasure>>,
    source: &str,
    id: &str,
    ingredients: &mut [Ingredient],
) {
    let Some(readings) = readings_by_recipe.get(&(source.to_owned(), id.to_owned())) else {
        return;
    };
    if readings.len() != ingredients.len() {
        return;
    }
    for (ing, reading) in ingredients.iter_mut().zip(readings) {
        ing.structured = Some(reading.clone());
    }
}

/// The `(source, id)` of every recipe that already has a stored reading.
async fn stored_recipes(conn: &Connection) -> anyhow::Result<HashSet<RecipeKey>> {
    let mut rows = conn
        .query("SELECT source, id FROM ingredient_structures", ())
        .await?;
    let mut set = HashSet::new();
    while let Some(row) = rows.next().await? {
        set.insert((row.get::<String>(0)?, row.get::<String>(1)?));
    }
    Ok(set)
}

/// Every recipe's `(source, id, ingredients)`, read from the derived view. A row
/// whose JSON no longer parses yields an empty ingredient list (skipped) rather
/// than failing the run.
async fn recipe_ingredients(
    conn: &Connection,
) -> anyhow::Result<Vec<(String, String, Vec<Ingredient>)>> {
    let mut rows = conn
        .query("SELECT source, id, ingredients FROM recipes", ())
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let json: String = row.get(2)?;
        out.push((
            source,
            id,
            serde_json::from_str::<Vec<Ingredient>>(&json).unwrap_or_default(),
        ));
    }
    Ok(out)
}

/// Write one recipe's readings, keyed by `(source, id)`, stamped with the model.
async fn store(
    conn: &Connection,
    source: &str,
    id: &str,
    readings: &[StructuredMeasure],
    model: &str,
) -> anyhow::Result<()> {
    let structured = serde_json::to_string(readings)?;
    conn.execute(
        "INSERT INTO ingredient_structures (source, id, structured, model)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(source, id) DO UPDATE SET
            structured = excluded.structured,
            model      = excluded.model,
            created_at = unixepoch()",
        libsql::params![source, id, structured, model],
    )
    .await?;
    Ok(())
}

// --- The production extractor: any OpenAI-compatible chat-completions endpoint. -

/// Guardrails for the extraction. The schema constrains the *shape*; this
/// constrains the *reading* — most importantly, not inventing ingredients.
const SYSTEM_PROMPT: &str = "\
You normalize cooking-recipe ingredient lines into structured data. You are given \
a JSON array of lines, each with a `name` and an optional `measure` (the raw \
quantity text, exactly as the source wrote it). Return one reading per line, in \
the same order, matching the provided schema.

Rules:
- `item` is the ingredient itself, taken from the line — never invent an \
  ingredient the line does not name. If `name` is the ingredient, use it; if the \
  item is folded into the measure text (name empty, measure \"1 cup flour\"), pull \
  the item out of it.
- `amount` is null when the line states no quantity at all.
- A plain number is `{kind: exact}`. A range like \"2-3\" is `{kind: range}`.
- A phrase with no number — \"to taste\", \"a pinch\", \"a splash\" — is a \
  qualitative amount. Do not invent a number for it.
- Put preparation (\"minced\", \"finely chopped\") in `preparation`, and anything \
  else the line carries (\"to serve\", \"optional\", \"plus extra\") in `note`. \
  Leave each null when absent.
- A size annotation like \"1 (14 oz) can\" is quantity 1, unit \"can\", size \
  {quantity 14, unit \"oz\"}.
- Do not convert units or do any arithmetic — record what the line says. \
  Conversion and scaling happen deterministically downstream.";

/// The production [`Extractor`]: one call per recipe to an OpenAI-compatible
/// `/chat/completions` endpoint. Rust has no official Anthropic SDK — and the point
/// is not to need one: `base_url`, `model`, and an optional `api_key` are all
/// per-deployment config, so the same code targets OpenAI, OpenRouter, Together,
/// Groq, or a local Ollama/vLLM without a change. `Clone` so it can live in the
/// shared `AppState` (the inner `reqwest::Client` is `Arc`-backed).
#[derive(Clone)]
pub struct OpenAiCompatExtractor {
    http: reqwest::Client,
    /// The API root, e.g. `https://api.openai.com/v1` — `/chat/completions` is
    /// appended. Trailing slash trimmed so the join never doubles up.
    base_url: String,
    /// `None` for a keyless endpoint (a local model). Sent as `Authorization:
    /// Bearer` when present.
    api_key: Option<String>,
    model: String,
}

impl OpenAiCompatExtractor {
    /// Build from the environment, or `None` when enrichment is not configured —
    /// the caller then skips it rather than failing.
    ///
    /// Active iff **both** `LLM_BASE_URL` and `LLM_MODEL` are set: there is no
    /// sensible universal default for either (the model depends on the endpoint),
    /// and defaulting the URL to one vendor is exactly the lock-in this design
    /// avoids. `LLM_API_KEY` is optional — a local endpoint needs none, and an
    /// empty one is filtered so it can't send a blank `Bearer`.
    pub fn from_env() -> Option<Self> {
        let base_url = non_empty_env("LLM_BASE_URL")?;
        let model = non_empty_env("LLM_MODEL")?;
        let api_key = non_empty_env("LLM_API_KEY");
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .ok()?;
        Some(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
        })
    }
}

/// An env var, trimmed, or `None` when unset or empty.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

impl Extractor for OpenAiCompatExtractor {
    async fn extract(&self, lines: &[Ingredient]) -> anyhow::Result<Vec<StructuredMeasure>> {
        if lines.is_empty() {
            return Ok(vec![]);
        }
        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request_body(&self.model, lines));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("llm endpoint responded {status}: {text}");
        }
        parse_response(&text, lines.len())
    }

    fn provenance(&self) -> String {
        self.model.clone()
    }
}

/// The chat-completions request: the lines as JSON, and the readings schema as a
/// strict `json_schema` response format so the reply is always valid,
/// deserializable JSON. `max_tokens` (not `max_completion_tokens`) for the widest
/// compatibility across OpenAI-compatible endpoints.
fn request_body(model: &str, lines: &[Ingredient]) -> Value {
    let numbered: Vec<Value> = lines
        .iter()
        .map(|l| json!({ "name": l.name, "measure": l.measure }))
        .collect();
    json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            {
                "role": "user",
                "content": format!(
                    "Read these {n} ingredient lines into structure. Return exactly \
                     {n} readings, in this order:\n{}",
                    serde_json::to_string_pretty(&numbered).unwrap_or_default(),
                    n = lines.len(),
                ),
            },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "ingredient_readings",
                "strict": true,
                "schema": readings_schema(),
            },
        },
    })
}

/// Pull the readings out of a chat-completions response and check the count. A
/// refusal, an empty/non-text reply, or a count mismatch is an error the engine
/// records as a failed recipe (it stays unenriched).
fn parse_response(text: &str, expected: usize) -> anyhow::Result<Vec<StructuredMeasure>> {
    let resp: ChatResponse = serde_json::from_str(text)?;
    let choice = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no choices in the response"))?;
    if let Some(refusal) = choice.message.refusal {
        anyhow::bail!("model refused the extraction: {refusal}");
    }
    let content = choice
        .message
        .content
        .ok_or_else(|| anyhow::anyhow!("no content in the response message"))?;
    let readings: Readings = serde_json::from_str(&content)?;
    if readings.readings.len() != expected {
        anyhow::bail!(
            "expected {expected} readings, got {}",
            readings.readings.len()
        );
    }
    Ok(readings.readings)
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    /// The model's text — the JSON we asked for. `null` on a refusal.
    content: Option<String>,
    /// Present (with a reason) when the model declined under structured outputs.
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Deserialize)]
struct Readings {
    readings: Vec<StructuredMeasure>,
}

// --- The JSON schema, mirroring the serde representation of StructuredMeasure. --
// Built by hand rather than derived because the wire schema and the Rust type are
// two sides of the same contract; a mismatch shows up immediately as a parse
// failure in the round-trip test below. The type is non-recursive (Size is flat),
// which is what lets it be a structured-output schema at all.

/// `{ "readings": [StructuredMeasure, ...] }`.
fn readings_schema() -> Value {
    obj(&[(
        "readings",
        json!({ "type": "array", "items": structured_measure_schema() }),
    )])
}

fn structured_measure_schema() -> Value {
    obj(&[
        ("item", json!({ "type": "string" })),
        ("amount", nullable(amount_schema())),
        ("preparation", nullable(json!({ "type": "string" }))),
        ("note", nullable(json!({ "type": "string" }))),
    ])
}

fn amount_schema() -> Value {
    json!({ "anyOf": [
        obj(&[
            ("kind", tag("quantified")),
            ("quantity", quantity_schema()),
            ("unit", nullable(json!({ "type": "string" }))),
            ("size", nullable(size_schema())),
        ]),
        obj(&[
            ("kind", tag("qualitative")),
            ("text", json!({ "type": "string" })),
        ]),
    ]})
}

fn quantity_schema() -> Value {
    json!({ "anyOf": [
        obj(&[
            ("kind", tag("exact")),
            ("value", json!({ "type": "number" })),
        ]),
        obj(&[
            ("kind", tag("range")),
            ("low", json!({ "type": "number" })),
            ("high", json!({ "type": "number" })),
        ]),
    ]})
}

fn size_schema() -> Value {
    obj(&[
        ("quantity", quantity_schema()),
        ("unit", nullable(json!({ "type": "string" }))),
    ])
}

/// A closed object schema: every listed property required, nothing else allowed —
/// the form strict structured output wants.
fn obj(props: &[(&str, Value)]) -> Value {
    let required: Vec<&str> = props.iter().map(|(k, _)| *k).collect();
    let properties: serde_json::Map<String, Value> = props
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

/// A `#[serde(tag = "kind")]` discriminator, as a single-value `enum` rather than
/// `const` — `enum` is in the strict-mode subset every OpenAI-compatible endpoint
/// supports, whereas `const` is not universal.
fn tag(value: &str) -> Value {
    json!({ "enum": [value] })
}

/// `T | null`, for the `Option<_>` fields (which serialize as `null`, not absent).
fn nullable(schema: Value) -> Value {
    json!({ "anyOf": [schema, { "type": "null" }] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::measure::{Amount, Quantity, Size};
    use std::sync::Mutex;

    fn ing(name: &str, measure: Option<&str>) -> Ingredient {
        Ingredient {
            name: name.into(),
            measure: measure.map(str::to_owned),
            structured: None,
        }
    }

    /// A reading of a line as just its item — enough to prove plumbing without
    /// modelling real extraction.
    fn item_reading(name: &str) -> StructuredMeasure {
        StructuredMeasure {
            item: name.into(),
            amount: None,
            preparation: None,
            note: None,
        }
    }

    /// An extractor that reads each line's name as the item and records every batch
    /// it was asked to read — so a test can assert what actually hit "the model".
    #[derive(Default)]
    struct SpyExtractor {
        batches: Mutex<Vec<Vec<Ingredient>>>,
    }

    impl Extractor for SpyExtractor {
        async fn extract(&self, lines: &[Ingredient]) -> anyhow::Result<Vec<StructuredMeasure>> {
            self.batches.lock().unwrap().push(lines.to_vec());
            Ok(lines.iter().map(|l| item_reading(&l.name)).collect())
        }

        fn provenance(&self) -> String {
            "spy-model".into()
        }
    }

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    async fn insert_recipe(conn: &Connection, id: &str, ingredients: &[Ingredient]) {
        let json = serde_json::to_string(ingredients).unwrap();
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', ?1, 'T', ?2, 'go')",
            libsql::params![id, json],
        )
        .await
        .unwrap();
    }

    /// The happy path: a recipe with no reading yet gets one row, the whole
    /// ingredient list read in a single call; the report counts recipes.
    #[tokio::test]
    async fn enriches_a_recipe_into_one_row() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            &[ing("flour", Some("1 cup")), ing("salt", None)],
        )
        .await;

        let spy = SpyExtractor::default();
        let report = enrich(&conn, &spy, false).await.unwrap();

        assert_eq!(
            report,
            EnrichReport {
                missing: 1,
                enriched: 1,
                failed: 0
            }
        );
        let loaded = load(&conn).await.unwrap();
        let readings = loaded.get(&("themealdb".into(), "1".into())).unwrap();
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].item, "flour");

        // One call carrying BOTH lines — per recipe, not per line. Locked last so
        // the guard isn't held across the await above.
        let batches = spy.batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    /// Each row records which model produced it — provenance for a
    /// non-deterministic, drifting source.
    #[tokio::test]
    async fn records_the_model_provenance() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("flour", Some("1 cup"))]).await;

        enrich(&conn, &SpyExtractor::default(), false)
            .await
            .unwrap();

        let mut rows = conn
            .query("SELECT model FROM ingredient_structures WHERE id = '1'", ())
            .await
            .unwrap();
        let model: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(model, "spy-model");
    }

    /// A second run does nothing: the recipe already has a row. Idempotent, so the
    /// scheduled pipeline only pays for genuinely new recipes.
    #[tokio::test]
    async fn a_second_run_skips_stored_recipes() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("flour", Some("1 cup"))]).await;

        let spy = SpyExtractor::default();
        enrich(&conn, &spy, false).await.unwrap();
        let second = enrich(&conn, &spy, false).await.unwrap();

        assert_eq!(
            second,
            EnrichReport {
                missing: 0,
                enriched: 0,
                failed: 0
            }
        );
        assert_eq!(spy.batches.lock().unwrap().len(), 1, "no second call");
    }

    /// `refresh` re-reads a recipe even when already stored — the deliberate
    /// re-snapshot after a model change; the routine run does not.
    #[tokio::test]
    async fn refresh_recaptures_a_stored_recipe() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("flour", Some("1 cup"))]).await;

        let spy = SpyExtractor::default();
        enrich(&conn, &spy, false).await.unwrap();
        let report = enrich(&conn, &spy, true).await.unwrap();

        assert_eq!(report.enriched, 1, "refresh re-reads the stored recipe");
        assert_eq!(
            spy.batches.lock().unwrap().len(),
            2,
            "read again on refresh"
        );
    }

    /// A wrong reading count is a failed recipe, not a misalignment — no row is
    /// written, so the recipe stays unenriched rather than storing wrong readings.
    #[tokio::test]
    async fn a_count_mismatch_fails_without_storing() {
        struct MiscountExtractor;
        impl Extractor for MiscountExtractor {
            async fn extract(
                &self,
                _lines: &[Ingredient],
            ) -> anyhow::Result<Vec<StructuredMeasure>> {
                Ok(vec![item_reading("only one")]) // fewer than asked
            }
            fn provenance(&self) -> String {
                "miscount".into()
            }
        }
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("a", None), ing("b", None)]).await;

        let report = enrich(&conn, &MiscountExtractor, false).await.unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(report.enriched, 0);
        assert!(load(&conn).await.unwrap().is_empty());
    }

    /// `attach` is the join derive performs: a recipe's readings zip onto its
    /// ingredients; a recipe with no row stays `None`; a stored array whose count
    /// no longer matches (raw changed since) does not attach.
    #[test]
    fn attach_zips_matching_readings_and_leaves_the_rest() {
        let mut readings = HashMap::new();
        readings.insert(
            ("themealdb".to_string(), "1".to_string()),
            vec![item_reading("flour"), item_reading("salt")],
        );
        // A stale row for recipe 2: one reading, but the recipe has two ingredients.
        readings.insert(
            ("themealdb".to_string(), "2".to_string()),
            vec![item_reading("only one")],
        );

        let mut r1 = vec![ing("flour", Some("1 cup")), ing("salt", None)];
        attach(&readings, "themealdb", "1", &mut r1);
        assert_eq!(
            r1[0].structured.as_ref().map(|m| &m.item),
            Some(&"flour".to_string())
        );
        assert_eq!(
            r1[1].structured.as_ref().map(|m| &m.item),
            Some(&"salt".to_string())
        );

        // Count mismatch → nothing attaches (re-enriches next run rather than misalign).
        let mut r2 = vec![ing("a", None), ing("b", None)];
        attach(&readings, "themealdb", "2", &mut r2);
        assert!(r2.iter().all(|i| i.structured.is_none()));

        // No row at all → None.
        let mut r3 = vec![ing("x", None)];
        attach(&readings, "themealdb", "9", &mut r3);
        assert_eq!(r3[0].structured, None);
    }

    /// The response parser pulls readings out of a chat-completions body and
    /// enforces the count.
    #[test]
    fn parse_response_reads_the_message_content_and_checks_count() {
        let reading = StructuredMeasure {
            item: "chopped tomatoes".into(),
            amount: Some(Amount::Quantified {
                quantity: Quantity::Exact { value: 1.0 },
                unit: Some("can".into()),
                size: Some(Size {
                    quantity: Quantity::Exact { value: 14.0 },
                    unit: Some("oz".into()),
                }),
            }),
            preparation: None,
            note: None,
        };
        let readings_json = serde_json::to_string(&json!({ "readings": [reading] })).unwrap();
        let body = json!({
            "choices": [{ "message": { "role": "assistant", "content": readings_json } }],
        })
        .to_string();

        let parsed = parse_response(&body, 1).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].item, "chopped tomatoes");
        // The wrong expected count is rejected.
        assert!(parse_response(&body, 2).is_err());
    }

    /// A refusal is an error, not a silent empty.
    #[test]
    fn parse_response_rejects_a_refusal() {
        let refusal = json!({
            "choices": [{ "message": { "role": "assistant", "content": null, "refusal": "no" } }],
        })
        .to_string();
        assert!(parse_response(&refusal, 1).is_err());
    }

    /// The schema mirrors the serde shape: a StructuredMeasure serialized to JSON
    /// validates against the generated schema. Guards the hand-built schema from
    /// drifting away from the type it constrains.
    #[test]
    fn the_schema_matches_the_serialized_type() {
        let sample = StructuredMeasure {
            item: "flour".into(),
            amount: Some(Amount::Quantified {
                quantity: Quantity::Range {
                    low: 2.0,
                    high: 3.0,
                },
                unit: Some("cup".into()),
                size: None,
            }),
            preparation: Some("sifted".into()),
            note: None,
        };
        let instance = serde_json::to_value(&sample).unwrap();
        assert!(
            matches_shape(&structured_measure_schema(), &instance),
            "instance {instance} does not match schema"
        );
    }

    /// A tiny structural check: object `required` keys exist, `anyOf` matches at
    /// least one branch, `enum` contains the value. Enough to catch a key or tag
    /// renamed on one side only, without a full JSON-Schema validator.
    fn matches_shape(schema: &Value, instance: &Value) -> bool {
        if schema.get("type").and_then(|t| t.as_str()) == Some("null") {
            return instance.is_null();
        }
        if let Some(branches) = schema.get("anyOf").and_then(|b| b.as_array()) {
            return branches.iter().any(|b| matches_shape(b, instance));
        }
        if let Some(values) = schema.get("enum").and_then(|e| e.as_array()) {
            return values.contains(instance);
        }
        match schema.get("type").and_then(|t| t.as_str()) {
            Some("object") => {
                let Some(inst_obj) = instance.as_object() else {
                    return false;
                };
                let props = schema.get("properties").and_then(|p| p.as_object());
                let required = schema.get("required").and_then(|r| r.as_array());
                if let (Some(props), Some(required)) = (props, required) {
                    required.iter().all(|k| {
                        let k = k.as_str().unwrap();
                        inst_obj
                            .get(k)
                            .is_some_and(|v| props.get(k).is_none_or(|sub| matches_shape(sub, v)))
                    })
                } else {
                    true
                }
            }
            Some("array") => instance.is_array(),
            Some("string") => instance.is_string(),
            Some("number") => instance.is_number(),
            _ => true,
        }
    }
}
