//! Shared recipe normalization used by both the native backend and the browser
//! (compiled to wasm32).
//!
//! Everything here is pure and synchronous — no network, no I/O. The caller
//! supplies raw bytes (the backend proxy fetches them, bypassing CORS) and
//! these functions turn them into the normalized [`models`] types. The same
//! logic therefore runs server-side and in-browser without duplication.

pub mod adapters;
pub mod models;
pub mod schema_org;
pub mod themealdb;

pub use adapters::{adapter_for, Adapter, UnsupportedSource};
pub use models::{Ingredient, Recipe};
