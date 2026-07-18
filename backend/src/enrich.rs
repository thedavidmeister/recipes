//! Enrich: read raw ingredient lines into structure (#11), and cache the result.
//!
//! This is the one **networked** stage of the corpus pipeline and the only writer
//! of `ingredient_structured`. It reads the lines the corpus holds (`recipes`),
//! finds the ones not yet cached, sends each recipe's uncached lines to an LLM in
//! one batched call, and writes the readings to the cache keyed by the raw line.
//! It does **not** write `recipes` — [`crate::derive`] reattaches from the cache,
//! which is why deriving stays offline.
//!
//! The LLM boundary is a trait ([`Extractor`]) so the engine runs against a
//! fixture with no network, the same shape [`crate::sync`] uses for fetch/store —
//! and so the **provider is not baked in**. Reading an ingredient line into JSON
//! is a commodity task, so production is [`OpenAiCompatExtractor`]: one call to
//! any OpenAI-compatible `/chat/completions` endpoint (OpenAI, OpenRouter,
//! Together, Groq, a local Ollama/vLLM, …), picked per deployment by env. The
//! reply is constrained to the [`StructuredMeasure`] schema by structured output,
//! so it is always valid JSON in the shape we deserialize.
//!
//! **Degrade-not-die.** With no endpoint configured there is no extractor, enrich
//! is a no-op, and derive leaves those lines `structured: None`. The corpus still
//! ingests and serves — enrichment is an addition, never a gate.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use libsql::Connection;
use recipe_core::{Ingredient, StructuredMeasure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The cache key for a raw line: `(name, measure-or-empty)`. `measure` is `""`
/// not `NULL` so it matches the table's `PRIMARY KEY (name, measure)` — SQLite
/// treats NULLs in a key as distinct, which would defeat the dedup.
fn key(ingredient: &Ingredient) -> (String, String) {
    (
        ingredient.name.clone(),
        ingredient.measure.clone().unwrap_or_default(),
    )
}

/// Reads a batch of raw ingredient lines into one [`StructuredMeasure`] each.
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
}

/// What an enrich run did.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct EnrichReport {
    /// Distinct uncached lines that needed a reading this run.
    pub missing: usize,
    /// Lines newly read and written to the cache.
    pub enriched: usize,
    /// Batches whose extraction failed — left uncached, so the line stays
    /// `structured: None` until a later run succeeds. A failure is never fatal.
    pub failed: usize,
}

/// Fill the structured cache for every ingredient line the corpus holds that is
/// not cached yet.
///
/// Reads `recipes` for the lines, batches each recipe's uncached lines into one
/// [`Extractor`] call, and writes the readings to `ingredient_structured`.
/// Idempotent: a line already in the cache — or already enriched earlier in this
/// same run, since recipes share lines — is never re-extracted.
pub async fn enrich<E: Extractor>(
    conn: &Connection,
    extractor: &E,
) -> anyhow::Result<EnrichReport> {
    let mut report = EnrichReport::default();
    // Lines we no longer need to read: everything already cached, growing as this
    // run caches more. Two recipes sharing a new line only pay for it once.
    let mut seen = cached_keys(conn).await?;

    for lines in read_recipe_lines(conn).await? {
        // This recipe's still-unseen lines, deduped within the recipe too.
        let mut batch: Vec<Ingredient> = Vec::new();
        for line in lines {
            let k = key(&line);
            if seen.contains(&k) || batch.iter().any(|b| key(b) == k) {
                continue;
            }
            batch.push(line);
        }
        if batch.is_empty() {
            continue;
        }
        report.missing += batch.len();

        match extractor.extract(&batch).await {
            Ok(readings) if readings.len() == batch.len() => {
                for (line, reading) in batch.iter().zip(&readings) {
                    cache_put(conn, line, reading).await?;
                    seen.insert(key(line));
                    report.enriched += 1;
                }
            }
            Ok(readings) => {
                tracing::warn!(
                    "extractor returned {} readings for {} lines — skipping batch",
                    readings.len(),
                    batch.len()
                );
                report.failed += 1;
            }
            Err(e) => {
                tracing::warn!("extraction failed for a batch of {}: {e}", batch.len());
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Load the whole structured cache into a map so [`crate::derive`] can reattach in
/// memory — one query, not a lookup per line across the corpus.
pub async fn load_cache(
    conn: &Connection,
) -> anyhow::Result<HashMap<(String, String), StructuredMeasure>> {
    let mut rows = conn
        .query(
            "SELECT name, measure, structured FROM ingredient_structured",
            (),
        )
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let name: String = row.get(0)?;
        let measure: String = row.get(1)?;
        let structured: String = row.get(2)?;
        // A row that no longer deserializes (a shape change) is skipped, not fatal
        // — the line simply stays unenriched until re-read.
        if let Ok(sm) = serde_json::from_str::<StructuredMeasure>(&structured) {
            map.insert((name, measure), sm);
        }
    }
    Ok(map)
}

/// Reattach cached readings onto a recipe's ingredients in place — the join
/// `derive` performs, offline, from the cache. A line with no cache entry is left
/// `None` (raw stays the source of truth).
pub fn attach(
    cache: &HashMap<(String, String), StructuredMeasure>,
    ingredients: &mut [Ingredient],
) {
    for ing in ingredients.iter_mut() {
        ing.structured = cache.get(&key(ing)).cloned();
    }
}

/// The `(name, measure)` keys already in the cache.
async fn cached_keys(conn: &Connection) -> anyhow::Result<HashSet<(String, String)>> {
    let mut rows = conn
        .query("SELECT name, measure FROM ingredient_structured", ())
        .await?;
    let mut set = HashSet::new();
    while let Some(row) = rows.next().await? {
        set.insert((row.get::<String>(0)?, row.get::<String>(1)?));
    }
    Ok(set)
}

/// Every recipe's ingredient list, read from the derived view. A row whose JSON
/// no longer parses yields an empty list rather than failing the run.
async fn read_recipe_lines(conn: &Connection) -> anyhow::Result<Vec<Vec<Ingredient>>> {
    let mut rows = conn.query("SELECT ingredients FROM recipes", ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let json: String = row.get(0)?;
        out.push(serde_json::from_str::<Vec<Ingredient>>(&json).unwrap_or_default());
    }
    Ok(out)
}

/// Write one line's reading to the cache, keyed by the raw line.
async fn cache_put(
    conn: &Connection,
    line: &Ingredient,
    reading: &StructuredMeasure,
) -> anyhow::Result<()> {
    let (name, measure) = key(line);
    let structured = serde_json::to_string(reading)?;
    conn.execute(
        "INSERT INTO ingredient_structured (name, measure, structured) VALUES (?1, ?2, ?3)
         ON CONFLICT(name, measure) DO UPDATE SET
            structured = excluded.structured,
            created_at = unixepoch()",
        libsql::params![name, measure, structured],
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

/// The production [`Extractor`]: one call per batch to an OpenAI-compatible
/// `/chat/completions` endpoint, constrained by a strict `json_schema` response
/// format so the reply is always valid, deserializable JSON.
///
/// Provider-neutral: `base_url`, `model`, and an optional `api_key` are all
/// per-deployment config, so the same code targets OpenAI, OpenRouter, Together,
/// Groq, or a local Ollama/vLLM without a change. `Clone` so it can live in the
/// shared `AppState` — the inner `reqwest::Client` is `Arc`-backed, so cloning is
/// cheap and shares one connection pool.
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
/// records as a failed batch (those lines stay unenriched).
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

    /// An extractor that echoes each line's name as the item and records every
    /// batch it was asked to read — so a test can assert what actually hit "the
    /// model", which is where dedup is proven.
    #[derive(Default)]
    struct SpyExtractor {
        batches: Mutex<Vec<Vec<Ingredient>>>,
    }

    impl Extractor for SpyExtractor {
        async fn extract(&self, lines: &[Ingredient]) -> anyhow::Result<Vec<StructuredMeasure>> {
            self.batches.lock().unwrap().push(lines.to_vec());
            Ok(lines.iter().map(|l| item_reading(&l.name)).collect())
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

    /// The happy path: uncached lines get read and written to the cache; the
    /// report counts them.
    #[tokio::test]
    async fn enriches_uncached_lines_into_the_cache() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            &[ing("flour", Some("1 cup")), ing("salt", None)],
        )
        .await;

        let spy = SpyExtractor::default();
        let report = enrich(&conn, &spy).await.unwrap();

        assert_eq!(
            report,
            EnrichReport {
                missing: 2,
                enriched: 2,
                failed: 0
            }
        );
        let cache = load_cache(&conn).await.unwrap();
        assert_eq!(
            cache
                .get(&("flour".into(), "1 cup".into()))
                .map(|m| &m.item),
            Some(&"flour".to_string())
        );
        // A line with no measure keys on the empty string.
        assert!(cache.contains_key(&("salt".into(), String::new())));
    }

    /// A line shared across recipes is read once — the dedup the line-keyed cache
    /// exists for. The spy proves the second recipe never reached the extractor.
    #[tokio::test]
    async fn a_shared_line_is_extracted_once() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("salt", Some("to taste"))]).await;
        insert_recipe(&conn, "2", &[ing("salt", Some("to taste"))]).await;

        let spy = SpyExtractor::default();
        let report = enrich(&conn, &spy).await.unwrap();

        assert_eq!(report.enriched, 1, "the shared line is read once");
        let batches = spy.batches.lock().unwrap();
        assert_eq!(
            batches.len(),
            1,
            "only one recipe's batch reached the model"
        );
    }

    /// A second run does nothing: everything is already cached. Idempotent, so the
    /// scheduled pipeline only ever pays for genuinely new lines.
    #[tokio::test]
    async fn a_second_run_extracts_nothing() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("flour", Some("1 cup"))]).await;

        let spy = SpyExtractor::default();
        enrich(&conn, &spy).await.unwrap();
        let second = enrich(&conn, &spy).await.unwrap();

        assert_eq!(
            second,
            EnrichReport {
                missing: 0,
                enriched: 0,
                failed: 0
            }
        );
    }

    /// A wrong reading count is a failed batch, not a misalignment — the lines
    /// stay uncached rather than getting the wrong readings.
    #[tokio::test]
    async fn a_count_mismatch_fails_the_batch_without_caching() {
        struct MiscountExtractor;
        impl Extractor for MiscountExtractor {
            async fn extract(
                &self,
                _lines: &[Ingredient],
            ) -> anyhow::Result<Vec<StructuredMeasure>> {
                Ok(vec![item_reading("only one")]) // fewer than asked
            }
        }
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("a", None), ing("b", None)]).await;

        let report = enrich(&conn, &MiscountExtractor).await.unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(report.enriched, 0);
        assert!(load_cache(&conn).await.unwrap().is_empty());
    }

    /// `attach` is the join derive performs: cached lines get their reading, an
    /// uncached line is left `None`.
    #[test]
    fn attach_reattaches_cached_readings_and_leaves_the_rest() {
        let mut cache = HashMap::new();
        cache.insert(
            ("flour".to_string(), "1 cup".to_string()),
            item_reading("flour"),
        );
        let mut ingredients = vec![ing("flour", Some("1 cup")), ing("pepper", None)];

        attach(&cache, &mut ingredients);

        assert_eq!(
            ingredients[0].structured.as_ref().map(|m| &m.item),
            Some(&"flour".to_string())
        );
        assert_eq!(
            ingredients[1].structured, None,
            "an uncached line stays None"
        );
    }

    /// The response parser pulls readings out of a chat-completions body and
    /// enforces the count.
    #[test]
    fn parse_response_reads_the_message_content_and_checks_count() {
        // A StructuredMeasure the way the model would return it under the schema.
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

        // The same body but the wrong expected count is rejected.
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
        // Every property the schema requires is present with an allowed shape.
        assert_shape(&structured_measure_schema(), &instance);
    }

    /// A tiny structural check: object `required` keys exist and `anyOf` matches at
    /// least one branch. Enough to catch a key or tag renamed on one side only,
    /// without pulling in a full JSON-Schema validator.
    fn assert_shape(schema: &Value, instance: &Value) {
        assert!(
            matches_shape(schema, instance),
            "instance {instance} does not match schema {schema}"
        );
    }

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
