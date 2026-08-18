//! The shared wire contract between `RadioCrate`'s backend and its generated
//! TypeScript client.
//!
//! Every request/response type that crosses the `POST /api/rpc` boundary is
//! defined here exactly once, so the backend (which `use`s these structs in
//! [`crate`]-consuming code like `backend::rpc`) and the client generator
//! (`cargo xtask gen-api`, which reads [`type_decls`] and [`METHODS`]) can never
//! drift apart.
//!
//! # The two casings
//!
//! The Rust code stays `snake_case` (the server's idiom, and what the DB columns
//! and SQL use); the *wire* is `camelCase`. Each struct carries
//! `#[serde(rename_all = "camelCase")]`, so serde renames the fields only at
//! (de)serialization — the transformation lives on the server, at the boundary.
//! `ts-rs` reads that same serde attribute (its default `serde-compat` feature),
//! so the generated `.ts` types match the wire exactly and the client needs no
//! conversion of its own.
//!
//! Because serde's `rename_all` renames only *declared* struct fields, the DML
//! method's free-form maps — keyed by *database column names* (see
//! `backend::dml`) — pass through verbatim with no special handling. DML's TS
//! types aren't modeled here; the generator emits them as a static block.

use serde::{Deserialize, Serialize};
use ts_rs::{Config, TS};

/// A saved query as exchanged over the wire. Timestamps are i64 epoch seconds
/// and are authored by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub modified_at: i64,
    #[ts(type = "number")]
    pub last_play: i64,
    pub definition: String,
}

/// A saved query-section preset as exchanged over the wire. The `section` is one
/// of `filter`/`sort`/`display` and the `definition` is a raw Querydown fragment;
/// both are opaque to the backend.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub base_table: String,
    pub section: String,
    pub definition: String,
    pub is_default: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub modified_at: i64,
}

/// A user override for a command's keyboard shortcut. `chord` is `None` when the
/// command is explicitly unbound. See `settings.keybinding` in migration 0002.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Keybinding {
    pub command_id: String,
    pub chord: Option<String>,
}

/// One user-customized setting: an opaque string `value` stored under a string
/// `key`. See `settings.settings` in migration 0004.
///
/// The backend never interprets either field. Which keys exist, what a value
/// means, and what the default is when no row exists all live in the frontend
/// (`frontend/src/state/settings.ts`), so a row is present only for a setting
/// the user has changed and deleting it restores the default.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    pub value: String,
}

/// The `build_id` a server reports when it embeds no frontend of its own — the
/// standalone dev server, which serves its client from Vite instead. A client
/// seeing this must skip the staleness comparison entirely rather than conclude
/// it is out of date, since there is no embedded build to be out of date
/// against. See [`AppVersion`].
pub const DEV_BUILD_ID: &str = "dev";

/// What the running server is, as reported by `app.version`.
///
/// `build_id` identifies the frontend build embedded in this binary (read from
/// `build-id.txt` in the embedded assets — see `frontend/vite.config.ts`), and
/// is the field that matters: a client compares it against the id compiled into
/// its own bundle, and the two differ exactly when the client running is not the
/// one this binary serves. The exception is [`DEV_BUILD_ID`], which means "don't
/// compare". `server_version` is the binary's own version string, for display
/// only — never for staleness decisions, since a rebuild at the same commit
/// leaves it unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppVersion {
    pub build_id: String,
    pub server_version: String,
}

/// Params for `query.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryDeleteParams {
    pub id: String,
}

/// Params for `query.record_play`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryRecordPlayParams {
    pub id: String,
    #[ts(type = "number")]
    pub last_play: i64,
}

/// Params for `query.rename`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryRenameParams {
    pub id: String,
    pub name: String,
}

/// Params for `query.update_definition`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryUpdateDefinitionParams {
    pub id: String,
    pub definition: String,
    #[ts(type = "number")]
    pub modified_at: i64,
}

/// Params for `preset.update`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresetUpdateParams {
    pub id: String,
    pub name: String,
    pub definition: String,
    pub is_default: bool,
    #[ts(type = "number")]
    pub modified_at: i64,
}

/// Params for `preset.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresetDeleteParams {
    pub id: String,
}

/// Params for `keybinding.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KeybindingDeleteParams {
    pub command_id: String,
}

/// Params for `setting.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingDeleteParams {
    pub key: String,
}

/// One method in the RPC surface, as consumed by the client generator.
///
/// The registry ([`METHODS`]) is the one hand-maintained list here; it names each
/// method, the generated client function, and the TS types on either side. The
/// TS type *definitions* still come from the `ts-rs` derives above (or, for DML,
/// the generator's static block), so this only wires names together.
pub struct Method {
    /// The JSON-RPC method name, e.g. `"query.list"`.
    pub wire: &'static str,
    /// The generated client function name, e.g. `"queryList"`.
    pub func: &'static str,
    /// The TS type of the single params argument, or `None` for a no-arg method.
    pub params: Option<&'static str>,
    /// The TS type the method resolves to, e.g. `"Query[]"` or `"null"`.
    pub result: &'static str,
}

/// Every `POST /api/rpc` method, in a stable order. Mirrors the dispatch in
/// `backend::rpc` and `backend::dml`.
pub const METHODS: &[Method] = &[
    Method {
        wire: "query.list",
        func: "queryList",
        params: None,
        result: "Query[]",
    },
    Method {
        wire: "query.add",
        func: "queryAdd",
        params: Some("Query"),
        result: "null",
    },
    Method {
        wire: "query.delete",
        func: "queryDelete",
        params: Some("QueryDeleteParams"),
        result: "null",
    },
    Method {
        wire: "query.record_play",
        func: "queryRecordPlay",
        params: Some("QueryRecordPlayParams"),
        result: "null",
    },
    Method {
        wire: "query.rename",
        func: "queryRename",
        params: Some("QueryRenameParams"),
        result: "null",
    },
    Method {
        wire: "query.update_definition",
        func: "queryUpdateDefinition",
        params: Some("QueryUpdateDefinitionParams"),
        result: "null",
    },
    Method {
        wire: "preset.list",
        func: "presetList",
        params: None,
        result: "Preset[]",
    },
    Method {
        wire: "preset.add",
        func: "presetAdd",
        params: Some("Preset"),
        result: "null",
    },
    Method {
        wire: "preset.update",
        func: "presetUpdate",
        params: Some("PresetUpdateParams"),
        result: "null",
    },
    Method {
        wire: "preset.delete",
        func: "presetDelete",
        params: Some("PresetDeleteParams"),
        result: "null",
    },
    Method {
        wire: "keybinding.list",
        func: "keybindingList",
        params: None,
        result: "Keybinding[]",
    },
    Method {
        wire: "keybinding.set",
        func: "keybindingSet",
        params: Some("Keybinding"),
        result: "null",
    },
    Method {
        wire: "keybinding.delete",
        func: "keybindingDelete",
        params: Some("KeybindingDeleteParams"),
        result: "null",
    },
    Method {
        wire: "setting.list",
        func: "settingList",
        params: None,
        result: "Setting[]",
    },
    Method {
        wire: "setting.set",
        func: "settingSet",
        params: Some("Setting"),
        result: "null",
    },
    Method {
        wire: "setting.delete",
        func: "settingDelete",
        params: Some("SettingDeleteParams"),
        result: "null",
    },
    Method {
        wire: "dml",
        func: "dml",
        params: Some("DmlRequest"),
        result: "DmlResult",
    },
    Method {
        wire: "app.version",
        func: "appVersion",
        params: None,
        result: "AppVersion",
    },
];

/// The `export type … = …;` declaration for every `ts-rs`-modeled wire type, in
/// the order the generated `types.ts` should list them. The DML types are *not*
/// here — the generator prepends them as a static block (see the module docs).
#[must_use]
pub fn type_decls() -> Vec<String> {
    // `TS::decl()` yields `type Name = { … };` with camelCased fields; we only add
    // the `export` keyword. Cross-type references (e.g. inside arrays) resolve by
    // name within the single generated file. The default `Config` is all we need —
    // per-field `#[ts(type = "number")]` overrides already pin the i64 timestamps,
    // so its `large_int` setting never applies to our types.
    let cfg = Config::default();
    vec![
        format!("export {}", Query::decl(&cfg)),
        format!("export {}", Preset::decl(&cfg)),
        format!("export {}", Keybinding::decl(&cfg)),
        format!("export {}", Setting::decl(&cfg)),
        format!("export {}", AppVersion::decl(&cfg)),
        format!("export {}", QueryDeleteParams::decl(&cfg)),
        format!("export {}", QueryRecordPlayParams::decl(&cfg)),
        format!("export {}", QueryRenameParams::decl(&cfg)),
        format!("export {}", QueryUpdateDefinitionParams::decl(&cfg)),
        format!("export {}", PresetUpdateParams::decl(&cfg)),
        format!("export {}", PresetDeleteParams::decl(&cfg)),
        format!("export {}", KeybindingDeleteParams::decl(&cfg)),
        format!("export {}", SettingDeleteParams::decl(&cfg)),
    ]
}
