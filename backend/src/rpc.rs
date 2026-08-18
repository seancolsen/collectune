//! Minimal JSON-RPC 2.0 endpoint for managing saved queries.
//!
//! The surface is tiny (a handful of methods over a single HTTP route), so the
//! envelope is hand-rolled with serde rather than pulling in a full RPC crate.
//! Each method runs on a blocking thread and talks to `DuckDB` through the
//! shared `Mutex<Connection>`, mirroring the other handlers in [`crate::server`].

use std::sync::Arc;

use api_schema::{
    AppVersion, Keybinding, KeybindingDeleteParams, Preset, PresetDeleteParams, PresetUpdateParams,
    Query, QueryDeleteParams, QueryRecordPlayParams, QueryRenameParams,
    QueryUpdateDefinitionParams, Setting, SettingDeleteParams,
};
use axum::Json;
use axum::extract::State;
use duckdb::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::AppState;

// The wire types (`Query`, `Preset`, `Keybinding`) and every method's params
// struct live in the shared `api-schema` crate, so the generated TypeScript
// client is derived from these exact definitions and can't drift. See that crate.

#[derive(Deserialize)]
pub(crate) struct RpcRequest {
    method: String,
    #[serde(default)]
    params: Value,
    id: Value,
}

#[derive(Serialize)]
pub(crate) struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
    id: Value,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
    /// Optional structured payload. Present (possibly `null`) for handlers that opt into structured
    /// errors — e.g. the `dml` handler reports which operation failed here; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// A handler error rich enough for the JSON-RPC `error` object: a numeric `code`, a `message`, and
/// an optional structured `data` payload.
///
/// Most handlers just produce a `String`; `From<String>` maps that to a generic server error with no
/// `data`. The `dml` handler builds these directly to attach an error `code` and a `data` object
/// naming the failing operation (see [`crate::dml`]).
#[derive(Debug)]
pub(crate) struct RpcErr {
    pub(crate) code: i32,
    pub(crate) message: String,
    pub(crate) data: Option<Value>,
}

impl From<String> for RpcErr {
    fn from(message: String) -> Self {
        RpcErr {
            code: -32000,
            message,
            data: None,
        }
    }
}

pub(crate) async fn rpc(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let id = req.id.clone();
    let outcome =
        tokio::task::spawn_blocking(move || dispatch(&state, &req.method, req.params)).await;

    match outcome {
        Ok(Ok(result)) => Json(RpcResponse {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }),
        Ok(Err(err)) => Json(RpcResponse {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError {
                code: err.code,
                message: err.message,
                data: err.data,
            }),
            id,
        }),
        Err(_) => Json(RpcResponse {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError {
                code: -32603,
                message: "rpc task panicked".to_string(),
                data: None,
            }),
            id,
        }),
    }
}

// Routes a method to its handler. The `dml` handler opts into the structured [`RpcErr`] error shape;
// every other method uses a plain `String` error, which `From<String>` widens.
fn dispatch(state: &AppState, method: &str, params: Value) -> Result<Value, RpcErr> {
    match method {
        "dml" => crate::dml::handle(state, params),
        _ => dispatch_legacy(state, method, params).map_err(RpcErr::from),
    }
}

// A flat match over the CRUD RPC methods; splitting it up would just scatter the
// per-method param structs and handlers.
#[allow(clippy::too_many_lines)]
fn dispatch_legacy(state: &AppState, method: &str, params: Value) -> Result<Value, String> {
    match method {
        // The one method here that touches no database: it answers from the
        // strings `app_state` was built with.
        "app.version" => serde_json::to_value(AppVersion {
            build_id: state.build_id.clone(),
            server_version: state.server_version.clone(),
        })
        .map_err(|e| e.to_string()),
        "query.list" => state.read(|conn| -> Result<Value, String> {
            let queries = list_queries(conn)?;
            serde_json::to_value(queries).map_err(|e| e.to_string())
        }),
        "query.add" => {
            let query: Query = from_params(params)?;
            state.write(|conn| {
                add_query(conn, &query)?;
                Ok(Value::Null)
            })
        }
        "query.delete" => {
            let p: QueryDeleteParams = from_params(params)?;
            state.write(|conn| {
                delete_query(conn, &p.id)?;
                Ok(Value::Null)
            })
        }
        "query.record_play" => {
            let p: QueryRecordPlayParams = from_params(params)?;
            state.write(|conn| {
                record_play(conn, &p.id, p.last_play)?;
                Ok(Value::Null)
            })
        }
        "query.rename" => {
            let p: QueryRenameParams = from_params(params)?;
            state.write(|conn| {
                rename_query(conn, &p.id, &p.name)?;
                Ok(Value::Null)
            })
        }
        "query.update_definition" => {
            let p: QueryUpdateDefinitionParams = from_params(params)?;
            state.write(|conn| {
                update_definition(conn, &p.id, &p.definition, p.modified_at)?;
                Ok(Value::Null)
            })
        }
        "preset.list" => state.read(|conn| -> Result<Value, String> {
            let presets = list_presets(conn)?;
            serde_json::to_value(presets).map_err(|e| e.to_string())
        }),
        "preset.add" => {
            let preset: Preset = from_params(params)?;
            state.write(|conn| {
                add_preset(conn, &preset)?;
                Ok(Value::Null)
            })
        }
        "preset.update" => {
            let p: PresetUpdateParams = from_params(params)?;
            state.write(|conn| {
                update_preset(
                    conn,
                    &p.id,
                    &p.name,
                    &p.definition,
                    p.is_default,
                    p.modified_at,
                )?;
                Ok(Value::Null)
            })
        }
        "preset.delete" => {
            let p: PresetDeleteParams = from_params(params)?;
            state.write(|conn| {
                delete_preset(conn, &p.id)?;
                Ok(Value::Null)
            })
        }
        "keybinding.list" => state.read(|conn| -> Result<Value, String> {
            let bindings = list_keybindings(conn)?;
            serde_json::to_value(bindings).map_err(|e| e.to_string())
        }),
        "keybinding.set" => {
            let binding: Keybinding = from_params(params)?;
            state.write(|conn| {
                set_keybinding(conn, &binding.command_id, binding.chord.as_deref())?;
                Ok(Value::Null)
            })
        }
        "keybinding.delete" => {
            let p: KeybindingDeleteParams = from_params(params)?;
            state.write(|conn| {
                delete_keybinding(conn, &p.command_id)?;
                Ok(Value::Null)
            })
        }
        // The settings key/value store. A row exists only for a setting the user
        // has customized, so `setting.delete` is how the frontend resets one to
        // the default it holds in code — see `api_schema::Setting`.
        "setting.list" => state.read(|conn| -> Result<Value, String> {
            let settings = list_settings(conn)?;
            serde_json::to_value(settings).map_err(|e| e.to_string())
        }),
        "setting.set" => {
            let setting: Setting = from_params(params)?;
            state.write(|conn| {
                set_setting(conn, &setting.key, &setting.value)?;
                Ok(Value::Null)
            })
        }
        "setting.delete" => {
            let p: SettingDeleteParams = from_params(params)?;
            state.write(|conn| {
                delete_setting(conn, &p.key)?;
                Ok(Value::Null)
            })
        }
        other => Err(format!("method not found: {other}")),
    }
}

fn from_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| e.to_string())
}

fn list_queries(conn: &Connection) -> Result<Vec<Query>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id::text, name, epoch(created_at)::bigint, \
             epoch(modified_at)::bigint, epoch(last_play)::bigint, definition \
             FROM query ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Query {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                modified_at: row.get(3)?,
                last_play: row.get(4)?,
                definition: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn add_query(conn: &Connection, query: &Query) -> Result<(), String> {
    conn.execute(
        "INSERT INTO query (id, name, created_at, modified_at, last_play, definition) \
         VALUES (TRY_CAST(? AS UUID), ?, make_timestamp(? * 1000000)::timestamp_s, \
         make_timestamp(? * 1000000)::timestamp_s, make_timestamp(? * 1000000)::timestamp_s, ?)",
        duckdb::params![
            query.id,
            query.name,
            query.created_at,
            query.modified_at,
            query.last_play,
            query.definition,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_query(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM query WHERE id = TRY_CAST(? AS UUID)",
        duckdb::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn record_play(conn: &Connection, id: &str, last_play: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE query SET last_play = make_timestamp(? * 1000000)::timestamp_s WHERE id = TRY_CAST(? AS UUID)",
        duckdb::params![last_play, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn rename_query(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE query SET name = ? WHERE id = TRY_CAST(? AS UUID)",
        duckdb::params![name, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_presets(conn: &Connection) -> Result<Vec<Preset>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id::text, name, base_table, section, definition, is_default, \
             epoch(created_at)::bigint, epoch(modified_at)::bigint \
             FROM preset ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Preset {
                id: row.get(0)?,
                name: row.get(1)?,
                base_table: row.get(2)?,
                section: row.get(3)?,
                definition: row.get(4)?,
                is_default: row.get(5)?,
                created_at: row.get(6)?,
                modified_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn add_preset(conn: &Connection, preset: &Preset) -> Result<(), String> {
    conn.execute(
        "INSERT INTO preset (id, name, base_table, section, definition, is_default, created_at, modified_at) \
         VALUES (TRY_CAST(? AS UUID), ?, ?, ?, ?, ?, make_timestamp(? * 1000000)::timestamp_s, \
         make_timestamp(? * 1000000)::timestamp_s)",
        duckdb::params![
            preset.id,
            preset.name,
            preset.base_table,
            preset.section,
            preset.definition,
            preset.is_default,
            preset.created_at,
            preset.modified_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn update_preset(
    conn: &Connection,
    id: &str,
    name: &str,
    definition: &str,
    is_default: bool,
    modified_at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE preset SET name = ?, definition = ?, is_default = ?, \
         modified_at = make_timestamp(? * 1000000)::timestamp_s \
         WHERE id = TRY_CAST(? AS UUID)",
        duckdb::params![name, definition, is_default, modified_at, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_preset(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM preset WHERE id = TRY_CAST(? AS UUID)",
        duckdb::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_keybindings(conn: &Connection) -> Result<Vec<Keybinding>, String> {
    let mut stmt = conn
        .prepare("SELECT command_id, chord FROM settings.keybinding ORDER BY command_id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Keybinding {
                command_id: row.get(0)?,
                chord: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn set_keybinding(conn: &Connection, command_id: &str, chord: Option<&str>) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings.keybinding (command_id, chord) VALUES (?, ?)",
        duckdb::params![command_id, chord],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_keybinding(conn: &Connection, command_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM settings.keybinding WHERE command_id = ?",
        duckdb::params![command_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_settings(conn: &Connection) -> Result<Vec<Setting>, String> {
    let mut stmt = conn
        .prepare("SELECT \"key\", \"value\" FROM settings.settings ORDER BY \"key\"")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Setting {
                key: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings.settings (\"key\", \"value\") VALUES (?, ?)",
        duckdb::params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_setting(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM settings.settings WHERE \"key\" = ?",
        duckdb::params![key],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn update_definition(
    conn: &Connection,
    id: &str,
    definition: &str,
    modified_at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE query SET definition = ?, modified_at = make_timestamp(? * 1000000)::timestamp_s \
         WHERE id = TRY_CAST(? AS UUID)",
        duckdb::params![definition, modified_at, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
