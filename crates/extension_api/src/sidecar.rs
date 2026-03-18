//! A module for working with extension sidecars over stdio JSON-RPC.

use crate::wit::zed::extension::sidecar;
use crate::{Result, serde_json};

pub fn call_json(
    method: &str,
    params_json: Option<&str>,
    timeout_ms: Option<u32>,
) -> Result<String> {
    sidecar::call(method, params_json, timeout_ms)
}

pub fn call(
    method: &str,
    params: Option<&serde_json::Value>,
    timeout_ms: Option<u32>,
) -> Result<serde_json::Value> {
    let params_json = params
        .map(|params| serde_json::to_string(params).map_err(|error| error.to_string()))
        .transpose()?;
    let response_json = sidecar::call(method, params_json.as_deref(), timeout_ms)?;
    serde_json::from_str(&response_json).map_err(|error| error.to_string())
}

pub fn close() -> Result<()> {
    sidecar::close()
}
