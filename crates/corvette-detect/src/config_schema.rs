//! The detection configuration schema document, embedded so the served copy is the tested copy.

/// Stable schema `$id`, equal to the path the schema route serves it at.
pub const SCHEMA_ID: &str = "/corvette/api/v1/detection/config/schema";
/// The schema document itself, byte-for-byte the file under `docs/design`.
pub const SCHEMA_JSON: &str = include_str!("../../../docs/design/detection-config.schema.json");
