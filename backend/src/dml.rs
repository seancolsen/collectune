//! The `dml` RPC method: arbitrary, introspection-driven writes to the database.
//!
//! A single request carries an ordered list of `insert` / `update` / `delete` operations, all run in
//! one transaction (see [`crate::server::AppState::write_mut`]). The handler introspects the live
//! schema at the start of every request and uses that — column nullability, unique constraints, and
//! the convention-inferred foreign-key graph — to validate and build each operation. Nothing about
//! the schema is hard-coded here; the same code works for user-added tables and columns.
//!
//! Deliberately "dumb" about referential integrity: it never cascades or nulls out references. A
//! delete simply fails if it would leave a dangling reference, and the client is responsible for
//! ordering a deletion tree leaves-first. See `plans/2026-07-dml-api.md` for the full contract.

use std::collections::{BTreeSet, HashMap};

use duckdb::Connection;
use duckdb::types::Value as DuckValue;
use introspection::{Column, Schema, Table};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::rpc::RpcErr;
use crate::server::AppState;

/// The `code` used for every DML error for now (an unknown/unclassified error). We may add specific
/// codes later.
const DML_ERROR_CODE: i32 = 1;

#[derive(Deserialize)]
struct DmlParams {
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "lowercase")]
enum Operation {
    Insert {
        id: String,
        table: String,
        values: Map<String, Value>,
    },
    Update {
        id: String,
        table: String,
        #[serde(rename = "where")]
        where_: Map<String, Value>,
        values: Map<String, Value>,
    },
    Delete {
        id: String,
        table: String,
        #[serde(rename = "where")]
        where_: Map<String, Value>,
    },
}

impl Operation {
    fn id(&self) -> &str {
        match self {
            Operation::Insert { id, .. }
            | Operation::Update { id, .. }
            | Operation::Delete { id, .. } => id,
        }
    }

    /// The operation's `values` map, or an empty map for `delete` (which has none). Used when
    /// scanning for operation-references, which may only appear in `values`.
    fn values(&self) -> Option<&Map<String, Value>> {
        match self {
            Operation::Insert { values, .. } | Operation::Update { values, .. } => Some(values),
            Operation::Delete { .. } => None,
        }
    }

    fn produces_result(&self) -> bool {
        !matches!(self, Operation::Delete { .. })
    }
}

/// Handles a `dml` RPC request: validate, then run every operation in one transaction.
pub(crate) fn handle(state: &AppState, params: Value) -> Result<Value, RpcErr> {
    let params: DmlParams = serde_json::from_value(params)
        .map_err(|e| validation_err(format!("invalid params: {e}")))?;
    validate_operations(&params.operations)?;
    state.write_mut(|conn| run(conn, &params.operations))
}

/// Introspects the schema, then runs the operations in order inside a single transaction, returning
/// the per-operation result object. The transaction rolls back on any error (via `Transaction`'s
/// drop) and only commits — and checkpoints, via `write_mut` — if every operation succeeds.
fn run(conn: &mut Connection, operations: &[Operation]) -> Result<Value, RpcErr> {
    let schema = introspect(conn)?;
    let tx = conn
        .transaction()
        .map_err(|e| RpcErr::from(e.to_string()))?;

    // op id -> returned row, for operations that produce a result. Doubles as the source for
    // resolving operation-references.
    let mut results: HashMap<String, Value> = HashMap::new();
    for op in operations {
        match op {
            Operation::Insert { id, table, values } => {
                let row = do_insert(&tx, &schema, id, table, values, &results)?;
                results.insert(id.clone(), row);
            }
            Operation::Update {
                id,
                table,
                where_,
                values,
            } => {
                let row = do_update(&tx, &schema, id, table, where_, values, &results)?;
                results.insert(id.clone(), row);
            }
            Operation::Delete { id, table, where_ } => {
                do_delete(&tx, &schema, id, table, where_)?;
            }
        }
    }

    tx.commit().map_err(|e| RpcErr::from(e.to_string()))?;

    // Emit results in operation order for stable output.
    let mut out = Map::new();
    for op in operations {
        if let Some(row) = results.remove(op.id()) {
            out.insert(op.id().to_string(), row);
        }
    }
    Ok(Value::Object(out))
}

fn introspect(conn: &Connection) -> Result<Schema, RpcErr> {
    let json: String = conn
        .query_row(introspection::introspection_sql(), [], |row| row.get(0))
        .map_err(|e| RpcErr::from(format!("schema introspection failed: {e}")))?;
    Schema::parse(&json).map_err(RpcErr::from)
}

// ---------------------------------------------------------------------------
// Pre-execution validation (structural; failures carry `data: null`)
// ---------------------------------------------------------------------------

/// Validates cross-operation structure before touching the database: operation ids are unique, and
/// every operation-reference targets a strictly-preceding operation that produces a result. These
/// are the failures the spec reports "during validation", so their error `data` is `null`.
fn validate_operations(operations: &[Operation]) -> Result<(), RpcErr> {
    let mut seen_ids: BTreeSet<&str> = BTreeSet::new();
    for op in operations {
        if !seen_ids.insert(op.id()) {
            return Err(validation_err(format!(
                "duplicate operation id \"{}\"",
                op.id()
            )));
        }
    }

    // As we walk the list, `preceding` holds the ids of operations already seen, mapped to whether
    // they produce a result. A reference is valid only if its target is already in this map.
    let mut preceding: HashMap<&str, bool> = HashMap::new();
    for op in operations {
        if let Some(values) = op.values() {
            for (column, value) in values {
                let Some(reference) = as_reference(value)? else {
                    continue;
                };
                match preceding.get(reference) {
                    None => {
                        return Err(validation_err(format!(
                            "operation \"{}\" references \"{reference}\" on column \"{column}\", \
                             which is not a strictly-preceding operation",
                            op.id()
                        )));
                    }
                    Some(false) => {
                        return Err(validation_err(format!(
                            "operation \"{}\" references \"{reference}\" on column \"{column}\", \
                             which produces no result (only insert/update can be referenced)",
                            op.id()
                        )));
                    }
                    Some(true) => {}
                }
            }
        }
        preceding.insert(op.id(), op.produces_result());
    }
    Ok(())
}

/// If `value` is an operation-reference (`{ "id": "<operation-id>" }`), returns the referenced id.
/// Returns `None` for a plain scalar. A JSON object of any other shape is a malformed reference,
/// which is a validation error.
fn as_reference(value: &Value) -> Result<Option<&str>, RpcErr> {
    let Value::Object(map) = value else {
        return Ok(None);
    };
    match map.get("id") {
        Some(Value::String(id)) if map.len() == 1 => Ok(Some(id)),
        _ => Err(validation_err(
            "a reference value must be an object of the form { \"id\": \"<operation-id>\" }"
                .to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Per-operation execution (failures carry `data.operation.id`)
// ---------------------------------------------------------------------------

fn do_insert(
    tx: &Connection,
    schema: &Schema,
    op_id: &str,
    table_name: &str,
    values: &Map<String, Value>,
    results: &HashMap<String, Value>,
) -> Result<Value, RpcErr> {
    let table = lookup_table(schema, table_name, op_id)?;
    if values.is_empty() {
        return Err(op_err(op_id, "insert has no values".to_string()));
    }

    // `columns` and `value_exprs` stay parallel: a provided column contributes a `?` placeholder
    // (with a matching bind); an auto-generated id column contributes a `gen_random_uuid()` literal.
    let mut columns = Vec::new();
    let mut value_exprs = Vec::new();
    let mut binds = Vec::new();
    for (column, value) in values {
        require_column(table, column, op_id)?;
        binds.push(resolve_value(
            schema, table_name, op_id, column, value, results,
        )?);
        columns.push(quote_ident(column));
        value_exprs.push("?".to_string());
    }

    // The database generates no ids for us — the schema declares `id uuid primary key` with no
    // default — so fill in any identifying UUID key column the client left out. This is what lets a
    // client insert a record and then reference its (server-generated) id from a later operation.
    for column in &table.columns {
        if is_autogenerated_id(table, column) && !values.contains_key(&column.name) {
            columns.push(quote_ident(&column.name));
            value_exprs.push("gen_random_uuid()".to_string());
        }
    }

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) RETURNING {}",
        quote_ident(table_name),
        columns.join(", "),
        value_exprs.join(", "),
        returning_list(table),
    );
    query_returning_row(tx, &sql, &binds, table, op_id)?
        .ok_or_else(|| op_err(op_id, "insert returned no row".to_string()))
}

/// Whether the DML API should auto-generate a value for `column` on insert: a `NOT NULL` `UUID`
/// column that is a table's single-column primary/unique key (i.e. an identifying id column). These
/// are the columns clients omit and expect the server to fill in and return.
fn is_autogenerated_id(table: &Table, column: &Column) -> bool {
    column
        .type_name
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("UUID"))
        && !column.nullable
        && table
            .unique_constraints
            .iter()
            .any(|constraint| constraint.len() == 1 && constraint[0] == column.name)
}

fn do_update(
    tx: &Connection,
    schema: &Schema,
    op_id: &str,
    table_name: &str,
    where_: &Map<String, Value>,
    values: &Map<String, Value>,
    results: &HashMap<String, Value>,
) -> Result<Value, RpcErr> {
    let table = lookup_table(schema, table_name, op_id)?;
    if values.is_empty() {
        return Err(op_err(op_id, "update has no values".to_string()));
    }
    validate_where(table, where_, op_id)?;

    let mut assignments = Vec::new();
    let mut binds = Vec::new();
    for (column, value) in values {
        require_column(table, column, op_id)?;
        binds.push(resolve_value(
            schema, table_name, op_id, column, value, results,
        )?);
        assignments.push(format!("{} = ?", quote_ident(column)));
    }

    let (where_sql, where_binds) = build_where(where_, op_id)?;
    binds.extend(where_binds);

    let sql = format!(
        "UPDATE {} SET {} WHERE {} RETURNING {}",
        quote_ident(table_name),
        assignments.join(", "),
        where_sql,
        returning_list(table),
    );
    query_returning_row(tx, &sql, &binds, table, op_id)?.ok_or_else(|| {
        op_err(
            op_id,
            format!("update matched no rows in table \"{table_name}\""),
        )
    })
}

fn do_delete(
    tx: &Connection,
    schema: &Schema,
    op_id: &str,
    table_name: &str,
    where_: &Map<String, Value>,
) -> Result<(), RpcErr> {
    let table = lookup_table(schema, table_name, op_id)?;
    validate_where(table, where_, op_id)?;
    let (where_sql, where_binds) = build_where(where_, op_id)?;

    // Dangling-reference check, evaluated against live transaction state: for every column that
    // references this table, fail if any still-present row points at a row we're about to delete.
    for link in schema.incoming_links(table_name) {
        let check_sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {from_table} f WHERE f.{from_column} IS NOT NULL \
             AND f.{from_column} IN (SELECT {to_column} FROM {del_table} WHERE {where_clause}))",
            from_table = quote_ident(&link.from_table),
            from_column = quote_ident(&link.from_column),
            to_column = quote_ident(&link.to_column),
            del_table = quote_ident(table_name),
            where_clause = where_sql,
        );
        let dangling: bool = tx
            .query_row(
                &check_sql,
                duckdb::params_from_iter(where_binds.iter()),
                |row| row.get(0),
            )
            .map_err(|e| op_err(op_id, e.to_string()))?;
        if dangling {
            return Err(op_err(
                op_id,
                format!(
                    "cannot delete from \"{table_name}\": a row in \"{}\" still references it via \
                     column \"{}\"",
                    link.from_table, link.from_column
                ),
            ));
        }
    }

    let delete_sql = format!(
        "DELETE FROM {} WHERE {}",
        quote_ident(table_name),
        where_sql
    );
    let affected = tx
        .execute(&delete_sql, duckdb::params_from_iter(where_binds.iter()))
        .map_err(|e| op_err(op_id, e.to_string()))?;
    if affected == 0 {
        return Err(op_err(
            op_id,
            format!("delete matched no rows in table \"{table_name}\""),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn lookup_table<'a>(
    schema: &'a Schema,
    table_name: &str,
    op_id: &str,
) -> Result<&'a Table, RpcErr> {
    schema
        .table(table_name)
        .ok_or_else(|| op_err(op_id, format!("no such table \"{table_name}\"")))
}

fn require_column(table: &Table, column: &str, op_id: &str) -> Result<(), RpcErr> {
    if table.column(column).is_none() {
        return Err(op_err(
            op_id,
            format!("no such column \"{column}\" on table \"{}\"", table.name),
        ));
    }
    Ok(())
}

/// Validates that `where_` identifies exactly one row: its column set must equal some UNIQUE / PRIMARY
/// KEY constraint whose columns are all `NOT NULL`. (A nullable unique constraint can't guarantee a
/// single match, so we reject it.)
fn validate_where(table: &Table, where_: &Map<String, Value>, op_id: &str) -> Result<(), RpcErr> {
    if where_.is_empty() {
        return Err(op_err(op_id, "\"where\" must not be empty".to_string()));
    }
    for column in where_.keys() {
        require_column(table, column, op_id)?;
    }
    let where_columns: BTreeSet<&str> = where_.keys().map(String::as_str).collect();
    let identifies_one_row = table.unique_constraints.iter().any(|constraint| {
        let constraint_columns: BTreeSet<&str> = constraint.iter().map(String::as_str).collect();
        constraint_columns == where_columns
            && constraint
                .iter()
                .all(|c| table.column(c).is_some_and(|col| !col.nullable))
    });
    if !identifies_one_row {
        return Err(op_err(
            op_id,
            format!(
                "\"where\" columns {where_columns:?} do not match a non-null unique constraint on \
                 table \"{}\"",
                table.name
            ),
        ));
    }
    Ok(())
}

/// Builds a `col = ? AND …` clause and its binds from a `where` map. Rejects references and nulls:
/// a `where` value must be a scalar literal that can actually match a row.
fn build_where(
    where_: &Map<String, Value>,
    op_id: &str,
) -> Result<(String, Vec<DuckValue>), RpcErr> {
    let mut clauses = Vec::new();
    let mut binds = Vec::new();
    for (column, value) in where_ {
        if value.is_object() {
            return Err(op_err(
                op_id,
                format!("\"where\" value for \"{column}\" must be a literal, not a reference"),
            ));
        }
        if value.is_null() {
            return Err(op_err(
                op_id,
                format!("\"where\" value for \"{column}\" must not be null"),
            ));
        }
        binds.push(json_scalar_to_duck(value).map_err(|e| op_err(op_id, e))?);
        clauses.push(format!("{} = ?", quote_ident(column)));
    }
    Ok((clauses.join(" AND "), binds))
}

/// Resolves a `values` entry to a bind value. A scalar is a literal; an object `{ "id": ... }` is an
/// operation-reference, valid only on a foreign-key column, resolving to the referenced record's id.
fn resolve_value(
    schema: &Schema,
    table_name: &str,
    op_id: &str,
    column: &str,
    value: &Value,
    results: &HashMap<String, Value>,
) -> Result<DuckValue, RpcErr> {
    match as_reference(value)? {
        None => json_scalar_to_duck(value).map_err(|e| op_err(op_id, e)),
        Some(reference) => {
            if schema.link(table_name, column).is_none() {
                return Err(op_err(
                    op_id,
                    format!(
                        "column \"{column}\" is not a foreign key; operation-references are only \
                         allowed on foreign-key columns"
                    ),
                ));
            }
            let referenced_id = results
                .get(reference)
                .and_then(|row| row.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    op_err(
                        op_id,
                        format!("reference \"{reference}\" did not produce an id"),
                    )
                })?;
            Ok(DuckValue::Text(referenced_id.to_string()))
        }
    }
}

/// The `RETURNING` column list for a table: every column, with `UUID` columns cast to text so they
/// serialize as strings (matching the ids clients send and reference).
fn returning_list(table: &Table) -> String {
    table
        .columns
        .iter()
        .map(|column| {
            let ident = quote_ident(&column.name);
            if column
                .type_name
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("UUID"))
            {
                format!("CAST({ident} AS TEXT) AS {ident}")
            } else {
                ident
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Runs a statement that `RETURNING`s the full row, reading the (at most one) result row into a JSON
/// object keyed by column name. Constraint violations surface here as the statement executes.
fn query_returning_row(
    tx: &Connection,
    sql: &str,
    binds: &[DuckValue],
    table: &Table,
    op_id: &str,
) -> Result<Option<Value>, RpcErr> {
    let mut stmt = tx.prepare(sql).map_err(|e| op_err(op_id, e.to_string()))?;
    let mut rows = stmt
        .query(duckdb::params_from_iter(binds.iter()))
        .map_err(|e| op_err(op_id, e.to_string()))?;
    let Some(row) = rows.next().map_err(|e| op_err(op_id, e.to_string()))? else {
        return Ok(None);
    };
    let mut object = Map::new();
    for (index, column) in table.columns.iter().enumerate() {
        let value: DuckValue = row.get(index).map_err(|e| op_err(op_id, e.to_string()))?;
        object.insert(column.name.clone(), duck_to_json(value));
    }
    Ok(Some(Value::Object(object)))
}

/// Quotes a SQL identifier, doubling any embedded quotes. Every table/column name we interpolate into
/// SQL goes through this; values are always bound as parameters, never interpolated.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Converts a scalar JSON value to a `DuckDB` bind value. String values bind as text and rely on
/// `DuckDB`'s implicit casts to the target column type (e.g. text → UUID, text → enum).
fn json_scalar_to_duck(value: &Value) -> Result<DuckValue, String> {
    Ok(match value {
        Value::Null => DuckValue::Null,
        Value::Bool(b) => DuckValue::Boolean(*b),
        Value::String(s) => DuckValue::Text(s.clone()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                DuckValue::BigInt(i)
            } else if let Some(u) = n.as_u64() {
                DuckValue::UBigInt(u)
            } else if let Some(f) = n.as_f64() {
                DuckValue::Double(f)
            } else {
                return Err(format!("unsupported numeric value: {n}"));
            }
        }
        Value::Array(_) | Value::Object(_) => {
            return Err("array and object literals are not supported as column values".to_string());
        }
    })
}

/// Converts a `DuckDB` value (from a `RETURNING` row) into JSON for the response.
// One arm per DuckDB type keeps the mapping readable and easy to audit against the schema, even where
// several arms share a body (e.g. every integer width widens the same way, and unrepresentable types
// collapse to null like the wildcard).
#[allow(clippy::match_same_arms)]
fn duck_to_json(value: DuckValue) -> Value {
    match value {
        DuckValue::Null => Value::Null,
        DuckValue::Boolean(b) => Value::Bool(b),
        DuckValue::TinyInt(n) => n.into(),
        DuckValue::SmallInt(n) => n.into(),
        DuckValue::Int(n) => n.into(),
        DuckValue::BigInt(n) => n.into(),
        DuckValue::UTinyInt(n) => n.into(),
        DuckValue::USmallInt(n) => n.into(),
        DuckValue::UInt(n) => n.into(),
        DuckValue::UBigInt(n) => n.into(),
        DuckValue::HugeInt(n) => {
            i64::try_from(n).map_or_else(|_| Value::String(n.to_string()), Into::into)
        }
        DuckValue::Float(f) => float_to_json(f64::from(f)),
        DuckValue::Double(f) => float_to_json(f),
        DuckValue::Decimal(d) => Value::String(d.to_string()),
        DuckValue::Timestamp(_, n) | DuckValue::Time64(_, n) => n.into(),
        DuckValue::Date32(n) => n.into(),
        DuckValue::Text(s) | DuckValue::Enum(s) => Value::String(s),
        DuckValue::Blob(bytes) => Value::String(to_hex(&bytes)),
        DuckValue::List(items) | DuckValue::Array(items) => {
            Value::Array(items.into_iter().map(duck_to_json).collect())
        }
        DuckValue::Struct(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), duck_to_json(v.clone())))
                .collect(),
        ),
        // Interval / Map / Union have no natural JSON form and don't appear in our schema.
        _ => Value::Null,
    }
}

/// JSON can't represent NaN/infinity, so those become `null`.
fn float_to_json(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(s, "{byte:02x}");
    }
    s
}

fn op_err(op_id: &str, message: String) -> RpcErr {
    RpcErr {
        code: DML_ERROR_CODE,
        message,
        data: Some(json!({ "operation": { "id": op_id } })),
    }
}

fn validation_err(message: String) -> RpcErr {
    RpcErr {
        code: DML_ERROR_CODE,
        message,
        data: Some(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::{Connection, params};

    const FILE1: &str = "11111111-1111-1111-1111-111111111111";
    const TRACK1: &str = "22222222-2222-2222-2222-222222222222";
    const ALBUM1: &str = "33333333-3333-3333-3333-333333333333";
    const ARTIST1: &str = "44444444-4444-4444-4444-444444444444";
    const MISSING: &str = "99999999-9999-9999-9999-999999999999";

    /// A fresh in-memory database with the real migration schema applied.
    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("migrations/0001.sql"))
            .unwrap();
        conn.execute_batch(include_str!("migrations/0002.sql"))
            .unwrap();
        conn.execute_batch(include_str!("migrations/0003.sql"))
            .unwrap();
        conn
    }

    /// Runs a DML request against `conn`, mirroring [`handle`] but without the `AppState`/checkpoint
    /// wrapper (an in-memory database needs no checkpoint).
    fn dml(conn: &mut Connection, params: Value) -> Result<Value, RpcErr> {
        let params: DmlParams = serde_json::from_value(params)
            .map_err(|e| validation_err(format!("invalid params: {e}")))?;
        validate_operations(&params.operations)?;
        run(conn, &params.operations)
    }

    fn insert_file(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO file (id, path, hash, size, format, duration, mtime, added) \
             VALUES (?, ?, ?, ?, ?, to_seconds(?), ?, now())",
            params![
                id,
                format!("/music/{id}.mp3"),
                vec![0u8, 1u8],
                1u32,
                "mp3",
                1.0f32,
                1i64
            ],
        )
        .unwrap();
    }

    fn insert_track(conn: &Connection, id: &str, file: &str, album: Option<&str>) {
        conn.execute(
            "INSERT INTO track (id, file, album, title) VALUES (?, ?, ?, ?)",
            params![id, file, album, "Track Title"],
        )
        .unwrap();
    }

    fn insert_album(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO album (id, title, year) VALUES (?, ?, ?)",
            params![id, "Abbey Road", 1969u16],
        )
        .unwrap();
    }

    fn insert_artist(conn: &Connection, id: &str, name: &str) {
        conn.execute(
            "INSERT INTO artist (id, name) VALUES (?, ?)",
            params![id, name],
        )
        .unwrap();
    }

    fn insert_credit(conn: &Connection, track: &str, artist: &str) {
        conn.execute(
            "INSERT INTO credit (track, artist, \"order\") VALUES (?, ?, ?)",
            params![track, artist, 1.0f32],
        )
        .unwrap();
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    // --- Insert -------------------------------------------------------------

    #[test]
    fn insert_returns_full_row() {
        let mut conn = setup();
        let out = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "a", "operation": "insert", "table": "artist", "values": { "name": "Beatles" } }
            ]}),
        )
        .unwrap();
        assert_eq!(out["a"]["name"], "Beatles");
        // The returned id is the database-generated UUID as a string.
        assert!(out["a"]["id"].as_str().unwrap().contains('-'));
        assert_eq!(
            count(&conn, "SELECT count(*) FROM artist WHERE name = 'Beatles'"),
            1
        );
    }

    #[test]
    fn insert_reference_resolves_to_preceding_insert_id() {
        let mut conn = setup();
        insert_file(&conn, FILE1);
        insert_track(&conn, TRACK1, FILE1, None);

        let out = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "art", "operation": "insert", "table": "artist", "values": { "name": "Beatles" } },
                { "id": "cr", "operation": "insert", "table": "credit",
                  "values": { "track": TRACK1, "artist": { "id": "art" }, "order": 1 } }
            ]}),
        )
        .unwrap();

        let new_artist_id = out["art"]["id"].as_str().unwrap();
        assert_eq!(out["cr"]["artist"], new_artist_id);
        assert_eq!(
            count(
                &conn,
                &format!("SELECT count(*) FROM credit WHERE artist = '{new_artist_id}'")
            ),
            1
        );
    }

    #[test]
    fn insert_uses_client_supplied_id_when_present() {
        let mut conn = setup();
        // The server auto-generates ids, but an explicitly supplied one takes precedence.
        let out = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "a", "operation": "insert", "table": "artist",
                  "values": { "id": ARTIST1, "name": "Beatles" } }
            ]}),
        )
        .unwrap();
        assert_eq!(out["a"]["id"], ARTIST1);
    }

    #[test]
    fn insert_duplicate_unique_name_fails() {
        let mut conn = setup();
        insert_artist(&conn, ARTIST1, "Beatles");
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "a", "operation": "insert", "table": "artist", "values": { "name": "Beatles" } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "a" } })));
    }

    #[test]
    fn insert_reference_on_non_fk_column_rejected() {
        let mut conn = setup();
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "al", "operation": "insert", "table": "album", "values": { "title": "X" } },
                // `title` is not a foreign key, so it can't take an operation-reference.
                { "id": "a", "operation": "insert", "table": "artist", "values": { "name": { "id": "al" } } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "a" } })));
    }

    // --- Update -------------------------------------------------------------

    #[test]
    fn update_by_primary_key_succeeds_and_returns_row() {
        let mut conn = setup();
        insert_artist(&conn, ARTIST1, "Beatles");
        let out = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "u", "operation": "update", "table": "artist",
                  "where": { "id": ARTIST1 }, "values": { "name": "The Beatles" } }
            ]}),
        )
        .unwrap();
        assert_eq!(out["u"]["name"], "The Beatles");
        assert_eq!(out["u"]["id"], ARTIST1);
    }

    #[test]
    fn update_by_alternate_unique_constraint_succeeds() {
        let mut conn = setup();
        insert_artist(&conn, ARTIST1, "Beatles");
        // `artist.name` is a NOT NULL UNIQUE constraint, so it's a valid `where` key.
        let out = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "u", "operation": "update", "table": "artist",
                  "where": { "name": "Beatles" }, "values": { "name": "The Beatles" } }
            ]}),
        )
        .unwrap();
        assert_eq!(out["u"]["name"], "The Beatles");
    }

    #[test]
    fn update_nonexistent_row_fails() {
        let mut conn = setup();
        insert_artist(&conn, ARTIST1, "Beatles");
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "u", "operation": "update", "table": "artist",
                  "where": { "id": MISSING }, "values": { "name": "Nobody" } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "u" } })));
    }

    #[test]
    fn update_where_not_unique_constraint_rejected() {
        let mut conn = setup();
        insert_file(&conn, FILE1);
        insert_track(&conn, TRACK1, FILE1, None);
        // `title` is not a unique constraint, so it can't identify a single row.
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "u", "operation": "update", "table": "track",
                  "where": { "title": "Track Title" }, "values": { "title": "Retitled" } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "u" } })));
    }

    // --- Delete: dangling-reference protection ------------------------------

    #[test]
    fn delete_album_with_referencing_tracks_rejected() {
        let mut conn = setup();
        insert_file(&conn, FILE1);
        insert_album(&conn, ALBUM1);
        insert_track(&conn, TRACK1, FILE1, Some(ALBUM1));
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "d", "operation": "delete", "table": "album", "where": { "id": ALBUM1 } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "d" } })));
        assert_eq!(
            count(&conn, "SELECT count(*) FROM album"),
            1,
            "delete rolled back"
        );
    }

    #[test]
    fn null_out_references_then_delete_album_succeeds() {
        let mut conn = setup();
        insert_file(&conn, FILE1);
        insert_album(&conn, ALBUM1);
        insert_track(&conn, TRACK1, FILE1, Some(ALBUM1));
        dml(
            &mut conn,
            json!({ "operations": [
                { "id": "u", "operation": "update", "table": "track",
                  "where": { "id": TRACK1 }, "values": { "album": null } },
                { "id": "d", "operation": "delete", "table": "album", "where": { "id": ALBUM1 } }
            ]}),
        )
        .unwrap();
        assert_eq!(count(&conn, "SELECT count(*) FROM album"), 0);
        assert_eq!(
            count(&conn, "SELECT count(*) FROM track WHERE album IS NULL"),
            1
        );
    }

    #[test]
    fn delete_file_referenced_by_track_rejected_but_ordered_delete_succeeds() {
        let mut conn = setup();
        insert_file(&conn, FILE1);
        insert_track(&conn, TRACK1, FILE1, None);

        // Deleting the file alone is rejected: the track still references it.
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "d", "operation": "delete", "table": "file", "where": { "id": FILE1 } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "d" } })));

        // Deleting the track first, then the file, succeeds.
        dml(
            &mut conn,
            json!({ "operations": [
                { "id": "dt", "operation": "delete", "table": "track", "where": { "id": TRACK1 } },
                { "id": "df", "operation": "delete", "table": "file", "where": { "id": FILE1 } }
            ]}),
        )
        .unwrap();
        assert_eq!(count(&conn, "SELECT count(*) FROM file"), 0);
        assert_eq!(count(&conn, "SELECT count(*) FROM track"), 0);
    }

    #[test]
    fn delete_artist_requires_deleting_credits_first() {
        let mut conn = setup();
        insert_file(&conn, FILE1);
        insert_track(&conn, TRACK1, FILE1, None);
        insert_artist(&conn, ARTIST1, "Beatles");
        insert_credit(&conn, TRACK1, ARTIST1);

        // Omitting the credit delete is rejected (the credit still references the artist).
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "d", "operation": "delete", "table": "artist", "where": { "id": ARTIST1 } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "d" } })));

        // Deleting the credit first, then the artist, succeeds.
        dml(
            &mut conn,
            json!({ "operations": [
                { "id": "dc", "operation": "delete", "table": "credit",
                  "where": { "track": TRACK1, "artist": ARTIST1 } },
                { "id": "da", "operation": "delete", "table": "artist", "where": { "id": ARTIST1 } }
            ]}),
        )
        .unwrap();
        assert_eq!(count(&conn, "SELECT count(*) FROM artist"), 0);
        assert_eq!(count(&conn, "SELECT count(*) FROM credit"), 0);
    }

    #[test]
    fn delete_matching_zero_rows_fails() {
        let mut conn = setup();
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "d", "operation": "delete", "table": "artist", "where": { "id": MISSING } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "d" } })));
    }

    // --- Cross-operation validation -----------------------------------------

    #[test]
    fn reference_to_later_operation_rejected_during_validation() {
        let mut conn = setup();
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "cr", "operation": "insert", "table": "credit",
                  "values": { "track": TRACK1, "artist": { "id": "art" } } },
                { "id": "art", "operation": "insert", "table": "artist", "values": { "name": "Beatles" } }
            ]}),
        )
        .unwrap_err();
        // Validation error, before any operation ran: data is null.
        assert_eq!(err.data, Some(Value::Null));
    }

    #[test]
    fn reference_to_nonexistent_operation_rejected_during_validation() {
        let mut conn = setup();
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "a", "operation": "insert", "table": "artist",
                  "values": { "name": { "id": "ghost" } } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(Value::Null));
    }

    #[test]
    fn reference_to_delete_operation_rejected_during_validation() {
        let mut conn = setup();
        insert_artist(&conn, ARTIST1, "Beatles");
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "d", "operation": "delete", "table": "artist", "where": { "id": ARTIST1 } },
                { "id": "a", "operation": "insert", "table": "artist",
                  "values": { "name": { "id": "d" } } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(Value::Null));
    }

    #[test]
    fn duplicate_operation_ids_rejected_during_validation() {
        let mut conn = setup();
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "x", "operation": "insert", "table": "artist", "values": { "name": "A" } },
                { "id": "x", "operation": "insert", "table": "artist", "values": { "name": "B" } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(Value::Null));
    }

    // --- Transaction atomicity ----------------------------------------------

    #[test]
    fn whole_request_rolls_back_when_one_operation_fails() {
        let mut conn = setup();
        insert_artist(&conn, ARTIST1, "Beatles");
        // First op would succeed; second violates the unique name constraint.
        let err = dml(
            &mut conn,
            json!({ "operations": [
                { "id": "ok", "operation": "insert", "table": "artist", "values": { "name": "Wings" } },
                { "id": "bad", "operation": "insert", "table": "artist", "values": { "name": "Beatles" } }
            ]}),
        )
        .unwrap_err();
        assert_eq!(err.data, Some(json!({ "operation": { "id": "bad" } })));
        // The first insert must have been rolled back.
        assert_eq!(
            count(&conn, "SELECT count(*) FROM artist WHERE name = 'Wings'"),
            0
        );
    }
}
