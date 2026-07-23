//! `recipe-backend mcp` — an MCP server exposing the enrichment worker's two
//! operations as typed tools (#59), so the enrich skill calls them directly instead
//! of shelling out to the CLI.
//!
//! It is the same worker as `enrich pull|push`: a thin stdio server over
//! [`crate::enrich_api::client`], holding no database connection — only the app's
//! URL and the machine key, from its environment. The model gets exactly two tools
//! (`enrich_pull`, `enrich_push`) and can reach the corpus only through the app's
//! validating endpoints.
//!
//! **stdout is the JSON-RPC channel.** Logging must go to **stderr** or it corrupts
//! the protocol — [`serve`] installs a stderr subscriber, and this is why the `mcp`
//! subcommand is dispatched before the binary's default (stdout) tracing init.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use tracing_subscriber::EnvFilter;

use crate::enrich_api::client;
use crate::equipment_api::client as equipment_client;
use crate::step_api::client as step_client;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EnrichPullParams {
    /// Maximum recipes to return. Omit for the server's default page size; the
    /// worker loops until the queue is empty regardless.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EnrichPushParams {
    /// The readings produced from the pulled lines: a JSON array with one entry per
    /// recipe, each `{ "source", "id", "readings": [StructuredMeasure, ...] }`, the
    /// readings in ingredient order (see the skill for the StructuredMeasure shape).
    /// Pass a native JSON array; a JSON-encoded string of one is also accepted. The
    /// app validates this before it writes anything.
    readings: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct StepPullParams {
    /// Maximum recipes to return. Omit for the server's default page size; the
    /// worker loops until the queue is empty regardless.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct StepPushParams {
    /// The step readings produced from the pulled methods: a JSON array with one entry
    /// per recipe, each `{ "source", "id", "steps": [StructuredStep, ...] }` (see the
    /// skill for the StructuredStep shape: id, text, kind, seconds, after). Pass a
    /// native JSON array; a JSON-encoded string of one is also accepted. The app
    /// validates the graph before it writes anything.
    readings: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EquipmentPullParams {
    /// Maximum recipes to return. Omit for the server's default page size; the
    /// worker loops until the queue is empty regardless.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EquipmentPushParams {
    /// The equipment readings: a JSON array with one entry per recipe, each
    /// `{ "source", "id", "equipment": [{ "item": "wok" }, ...] }`. Names must already
    /// be normalised — lowercase, trimmed, single-spaced — because a kitchen selects
    /// from this vocabulary and "Wok" would be a second, unmatchable entry beside
    /// "wok". The app refuses an unnormalised reading rather than repairing it.
    readings: serde_json::Value,
}

/// The enrichment worker as an MCP server. Holds only the tool router — its config
/// (the app URL, the key, the model) is read from the environment per call by
/// [`client`], exactly as the CLI does.
#[derive(Clone)]
pub struct Enricher {
    // Read by the `#[tool_handler]`-generated `ServerHandler` to route tool calls,
    // but that read is inside macro-expanded code the dead-code lint doesn't trace
    // (the rmcp examples blanket the file with `#![allow(dead_code)]` for the same
    // reason). Kept scoped to the field.
    #[allow(dead_code)]
    tool_router: ToolRouter<Enricher>,
}

impl Default for Enricher {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl Enricher {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "enrich_pull",
        description = "Get the recipes that still need a structured reading of their \
                       ingredient lines. Returns a JSON array of {source, id, \
                       ingredients:[{name, measure}]}; an empty array means the queue \
                       is drained."
    )]
    async fn enrich_pull(
        &self,
        Parameters(EnrichPullParams { limit }): Parameters<EnrichPullParams>,
    ) -> Result<CallToolResult, McpError> {
        match client::pull_pending(limit).await {
            Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            // A failure here (endpoint down, env missing) is the worker's to see and
            // stop on — surface it as a tool error with the message, not a crash.
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "enrich_pull failed: {e}"
            ))])),
        }
    }

    #[tool(
        name = "enrich_push",
        description = "Submit readings for one or more recipes. The app validates \
                       each (recipe exists, reading count matches the current \
                       ingredient list), stores them, and re-derives. Returns \
                       {accepted, derived, rejected}."
    )]
    async fn enrich_push(
        &self,
        Parameters(EnrichPushParams { readings }): Parameters<EnrichPushParams>,
    ) -> Result<CallToolResult, McpError> {
        match client::push_readings(readings).await {
            Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "enrich_push failed: {e}"
            ))])),
        }
    }

    #[tool(
        name = "equipment_pull",
        description = "Get the recipes that still need a reading of the equipment they \
                       require. Returns a JSON array of {source, id, instructions}; an \
                       empty array means the queue is drained."
    )]
    async fn equipment_pull(
        &self,
        Parameters(EquipmentPullParams { limit }): Parameters<EquipmentPullParams>,
    ) -> Result<CallToolResult, McpError> {
        match equipment_client::pull_pending(limit).await {
            Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "equipment_pull failed: {e}"
            ))])),
        }
    }

    #[tool(
        name = "equipment_push",
        description = "Submit equipment readings: each recipe's {item} list, covering \
                       preparation as well as cooking — a salad still needs a bowl, a \
                       knife and a board. Names must be normalised (lowercase, \
                       trimmed); the app refuses a reading that is not, and refuses an \
                       empty one. Returns {accepted, derived, rejected}."
    )]
    async fn equipment_push(
        &self,
        Parameters(EquipmentPushParams { readings }): Parameters<EquipmentPushParams>,
    ) -> Result<CallToolResult, McpError> {
        match equipment_client::push_readings(readings).await {
            Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "equipment_push failed: {e}"
            ))])),
        }
    }

    #[tool(
        name = "step_pull",
        description = "Get the recipes that still need a structured reading of their \
                       method. Returns a JSON array of {source, id, instructions, \
                       ingredients:[{name, measure, preparation}]}; an empty array \
                       means the queue is drained."
    )]
    async fn step_pull(
        &self,
        Parameters(StepPullParams { limit }): Parameters<StepPullParams>,
    ) -> Result<CallToolResult, McpError> {
        match step_client::pull_pending(limit).await {
            Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "step_pull failed: {e}"
            ))])),
        }
    }

    #[tool(
        name = "step_push",
        description = "Submit step readings for one or more recipes: each a DAG of \
                       {id, text, kind, seconds, after}. The app validates the graph \
                       (0-based ids, edges point to earlier steps), stores it, and \
                       re-derives. Returns {accepted, derived, rejected}."
    )]
    async fn step_push(
        &self,
        Parameters(StepPushParams { readings }): Parameters<StepPushParams>,
    ) -> Result<CallToolResult, McpError> {
        match step_client::push_readings(readings).await {
            Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "step_push failed: {e}"
            ))])),
        }
    }
}

#[tool_handler]
impl ServerHandler for Enricher {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Recipe corpus enrichment (#59): three queues — ingredient lines, \
                 methods, and required equipment. Pull the recipes that still need a \
                 reading, read them, push the readings back. The app validates and \
                 writes; these tools never touch the database."
                    .to_string(),
            )
    }
}

/// Boot the stdio MCP server and block until the client disconnects.
///
/// Installs a **stderr** tracing subscriber first: stdout carries the JSON-RPC
/// protocol, so anything logged there would corrupt it.
pub async fn serve() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "recipe_backend=info,rmcp=warn".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("recipes enrich MCP server starting on stdio");
    let service = Enricher::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
