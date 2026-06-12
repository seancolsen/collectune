//! JSON-RPC client for the saved-query API (`/rpc`).
//!
//! Mirrors the native/wasm split used by [`crate::http`]: requests run on a
//! background thread (native) or via `spawn_local` (wasm), and results are
//! handed back to the UI thread through shared state + `request_repaint`.

use std::sync::{Arc, Mutex};

use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::query_def::{QueryDefinition, Section, codec};

/// A saved query. Timestamps are i64 epoch seconds and are authored here on the
/// frontend, then persisted verbatim by the backend. The structured definition
/// travels (and is stored) as a JSON string in the `definition` field.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Query {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) created_at: i64,
    pub(crate) modified_at: i64,
    pub(crate) last_play: i64,
    #[serde(with = "codec")]
    pub(crate) definition: QueryDefinition,
}

/// A saved query-section preset: a named, reusable Querydown fragment scoped
/// to one base table and one query section.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Preset {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) base_table: String,
    pub(crate) section: Section,
    pub(crate) definition: String,
    pub(crate) created_at: i64,
    pub(crate) modified_at: i64,
}

/// Current wall-clock time as epoch seconds (local clock).
pub(crate) fn now_epoch() -> i64 {
    jiff::Zoned::now().timestamp().as_second()
}

/// Current local time formatted as RFC 3339 with minute precision, e.g.
/// `2026-06-04T15:30`. Used as the default name for a new query.
pub(crate) fn now_name() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M").to_string()
}

/// Fetches every saved query and stores the result in `out` for the UI to drain.
pub(crate) fn list_queries(out: Arc<Mutex<Option<Vec<Query>>>>, ctx: egui::Context) {
    dispatch_call("query.list", Value::Null, move |result| {
        if let Ok(value) = result
            && let Ok(list) = serde_json::from_value::<Vec<Query>>(value)
        {
            *out.lock().unwrap() = Some(list);
            ctx.request_repaint();
        }
    });
}

/// Inserts a new query. Fire-and-forget.
pub(crate) fn add_query(query: &Query) {
    let params = serde_json::to_value(query).unwrap_or(Value::Null);
    dispatch_call("query.add", params, |_| {});
}

/// Persists an edited definition along with a new `modified_at`. Fire-and-forget.
pub(crate) fn update_definition(id: Uuid, definition: &QueryDefinition, modified_at: i64) {
    let params =
        json!({ "id": id, "definition": definition.to_stored(), "modified_at": modified_at });
    dispatch_call("query.update_definition", params, |_| {});
}

/// Fetches every saved preset and stores the result in `out` for the UI to drain.
pub(crate) fn list_presets(out: Arc<Mutex<Option<Vec<Preset>>>>, ctx: egui::Context) {
    dispatch_call("preset.list", Value::Null, move |result| {
        if let Ok(value) = result
            && let Ok(list) = serde_json::from_value::<Vec<Preset>>(value)
        {
            *out.lock().unwrap() = Some(list);
            ctx.request_repaint();
        }
    });
}

/// Inserts a new preset. Fire-and-forget.
pub(crate) fn add_preset(preset: &Preset) {
    let params = serde_json::to_value(preset).unwrap_or(Value::Null);
    dispatch_call("preset.add", params, |_| {});
}

/// Persists an edited preset name/definition. Fire-and-forget.
pub(crate) fn update_preset(id: Uuid, name: &str, definition: &str, modified_at: i64) {
    let params = json!({
        "id": id,
        "name": name,
        "definition": definition,
        "modified_at": modified_at,
    });
    dispatch_call("preset.update", params, |_| {});
}

/// Deletes a saved preset. Fire-and-forget.
pub(crate) fn delete_preset(id: Uuid) {
    let params = json!({ "id": id });
    dispatch_call("preset.delete", params, |_| {});
}

/// Records that a query was used to play a song (updates `last_play` only).
/// Fire-and-forget.
pub(crate) fn record_play(id: Uuid, last_play: i64) {
    let params = json!({ "id": id, "last_play": last_play });
    dispatch_call("query.record_play", params, |_| {});
}

/// Renames a saved query. Fire-and-forget.
pub(crate) fn rename_query(id: Uuid, name: &str) {
    let params = json!({ "id": id, "name": name });
    dispatch_call("query.rename", params, |_| {});
}

/// Deletes a saved query. Fire-and-forget.
pub(crate) fn delete_query(id: Uuid) {
    let params = json!({ "id": id });
    dispatch_call("query.delete", params, |_| {});
}

fn extract_result(value: &Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("rpc error");
        return Err(message.to_string());
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(not(target_arch = "wasm32"))]
fn dispatch_call<D>(method: &'static str, params: Value, on_done: D)
where
    D: FnOnce(Result<Value, String>) + Send + 'static,
{
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        on_done(rt.block_on(post_rpc(method, params)));
    });
}

#[cfg(target_arch = "wasm32")]
fn dispatch_call<D>(method: &'static str, params: Value, on_done: D)
where
    D: FnOnce(Result<Value, String>) + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        on_done(post_rpc(method, params).await);
    });
}

fn request_body(method: &str, params: Value) -> String {
    // Built by hand (rather than `json!`) so `params` is moved in, not borrowed.
    let mut request = serde_json::Map::with_capacity(4);
    request.insert("jsonrpc".to_owned(), Value::from("2.0"));
    request.insert("method".to_owned(), Value::from(method));
    request.insert("params".to_owned(), params);
    request.insert("id".to_owned(), Value::from(1));
    Value::Object(request).to_string()
}

#[cfg(not(target_arch = "wasm32"))]
async fn post_rpc(method: &str, params: Value) -> Result<Value, String> {
    let url = format!("{}/rpc", crate::http::BASE);
    let resp = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .body(request_body(method, params))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let msg = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {msg}"));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    extract_result(&value)
}

#[cfg(target_arch = "wasm32")]
async fn post_rpc(method: &str, params: Value) -> Result<Value, String> {
    let url = format!("{}/rpc", crate::http::BASE);
    let resp = gloo_net::http::Request::post(&url)
        .header("content-type", "application/json")
        .body(request_body(method, params))
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        let status = resp.status();
        let msg = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {msg}"));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    extract_result(&value)
}
