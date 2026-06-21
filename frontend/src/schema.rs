//! Post-processing of Querydown's database introspection.
//!
//! Querydown can introspect a `DuckDB` database into the schema JSON its compiler
//! needs, but it derives table links from the database's foreign keys. Collectune's
//! databases don't declare foreign keys — we can't use them because of limitations
//! with the DML we perform — so Querydown's introspection finds no links.
//!
//! We recover links by convention: any `UUID` column whose name matches another
//! table's name is treated as a link to that table's `id` column. This is the same
//! rule the backend used to apply during its own (now-removed) introspection; here
//! it runs as a transformation over the JSON that Querydown's introspection emits,
//! replacing its (empty) `links` array with the convention-inferred one.

use std::collections::HashSet;

use querydown::{Dialect, DuckDB};
use serde_json::{Value, json};

/// The SQL that introspects a `DuckDB` database into Querydown-shaped schema JSON.
///
/// Querydown provides this query directly (per SQL dialect) so that Collectune stays
/// in lockstep with the schema JSON structure the compiler expects: when that
/// structure changes, the introspection SQL changes with it. The query returns a
/// single row with a single JSON-document cell, which the caller runs through the
/// query API and then enriches via [`add_inferred_links`].
pub(crate) fn introspection_sql() -> String {
    DuckDB().introspection_sql().to_string()
}

/// Adds convention-inferred table links to Querydown's introspected schema JSON.
///
/// Takes the JSON document produced by running Querydown's introspection SQL and
/// returns it with a `links` array populated by the column-name/type convention
/// described in the module docs. Every other part of the document is preserved
/// verbatim, so the schema continues to conform to whatever shape Querydown's
/// introspection produces.
pub(crate) fn add_inferred_links(introspection_json: &str) -> Result<String, String> {
    let mut root: Value = serde_json::from_str(introspection_json).map_err(|e| e.to_string())?;
    let tables = root
        .get("tables")
        .and_then(Value::as_array)
        .ok_or("Introspected schema JSON has no \"tables\" array.")?;

    let table_names: HashSet<&str> = tables
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .collect();

    let mut links = Vec::new();
    for table in tables {
        let Some(table_name) = table.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(columns) = table.get("columns").and_then(Value::as_array) else {
            continue;
        };
        for column in columns {
            let Some(column_name) = column.get("name").and_then(Value::as_str) else {
                continue;
            };
            let column_type = column
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if column_type.eq_ignore_ascii_case("UUID") && table_names.contains(column_name) {
                links.push(json!({
                    "from": { "table": table_name, "column": column_name },
                    "to": { "table": column_name, "column": "id" },
                    "unique": false,
                }));
            }
        }
    }

    root["links"] = Value::Array(links);
    serde_json::to_string(&root).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_link_from_uuid_column_matching_table_name() {
        let input = r#"{
            "tables": [
                { "name": "track", "columns": [{ "name": "id", "type": "UUID" }] },
                { "name": "credit", "columns": [
                    { "name": "id", "type": "UUID" },
                    { "name": "track", "type": "UUID" },
                    { "name": "ord", "type": "INTEGER" }
                ] }
            ]
        }"#;
        let out: Value = serde_json::from_str(&add_inferred_links(input).unwrap()).unwrap();
        let links = out["links"].as_array().unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["from"]["table"], "credit");
        assert_eq!(links[0]["from"]["column"], "track");
        assert_eq!(links[0]["to"]["table"], "track");
        assert_eq!(links[0]["to"]["column"], "id");
        assert_eq!(links[0]["unique"], false);
    }

    #[test]
    fn ignores_non_uuid_and_non_matching_columns() {
        let input = r#"{
            "tables": [
                { "name": "track", "columns": [
                    { "name": "id", "type": "UUID" },
                    { "name": "title", "type": "VARCHAR" },
                    { "name": "track", "type": "VARCHAR" }
                ] }
            ]
        }"#;
        let out: Value = serde_json::from_str(&add_inferred_links(input).unwrap()).unwrap();
        assert!(out["links"].as_array().unwrap().is_empty());
    }

    #[test]
    fn preserves_other_fields_and_overwrites_links() {
        let input = r#"{ "tables": [], "links": [{ "stale": true }], "extra": 1 }"#;
        let out: Value = serde_json::from_str(&add_inferred_links(input).unwrap()).unwrap();
        assert!(out["links"].as_array().unwrap().is_empty());
        assert_eq!(out["extra"], 1);
    }

    #[test]
    fn rejects_json_without_tables() {
        assert!(add_inferred_links(r#"{ "nope": 1 }"#).is_err());
    }
}
