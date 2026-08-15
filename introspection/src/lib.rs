//! Shared database introspection for `RadioCrate`'s front end and back end.
//!
//! Both halves of the app need to understand the database's structure by introspecting it at
//! runtime, and both must agree on that structure. The front end feeds it to the Querydown compiler;
//! the back end's DML API uses it to validate and build writes. To keep the two from drifting, the
//! introspection SQL and the post-processing that enriches its output both live here.
//!
//! # The SQL
//!
//! [`introspection_sql`] returns a query that, run against a `DuckDB` database, yields a single JSON
//! document describing every table and column in the `main` schema (see `resources/duckdb.sql`). It
//! began as a copy of Querydown's own introspection query and was extended with the nullability and
//! unique-constraint metadata the DML API needs.
//!
//! # Inferred links
//!
//! `RadioCrate` declares no foreign keys — `DuckDB` can't update rows referenced by an `FK` constraint —
//! so the introspection SQL finds no links and always emits an empty `links` array. We recover links
//! by convention: any `UUID` column whose name matches another table's name is treated as a link to
//! that table's `id` column. This inference is applied two ways from one shared implementation:
//!
//! - [`add_inferred_links`] rewrites the `links` array of the raw JSON document, for the front end to
//!   hand to the Querydown compiler.
//! - [`Schema::parse`] parses the whole document into a structured [`Schema`] (with the inferred
//!   links) for the back end.

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{Value, json};

/// The SQL that introspects a `DuckDB` database into `RadioCrate`'s schema JSON.
///
/// The query returns a single row with a single JSON-text cell. Run it through the query API, read
/// the lone value, then either enrich it with [`add_inferred_links`] (front end) or parse it with
/// [`Schema::parse`] (back end).
#[must_use]
pub fn introspection_sql() -> &'static str {
    include_str!("../resources/duckdb.sql")
}

/// A convention-inferred foreign-key link: `from_table.from_column` references `to_table.to_column`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
}

/// A single column and the metadata the DML API reasons about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// The column's `DuckDB` type name (e.g. `"UUID"`, `"VARCHAR"`), if the introspection provided one.
    pub type_name: Option<String>,
    /// Whether the column permits `NULL`.
    pub nullable: bool,
}

/// A single table: its columns and its UNIQUE / PRIMARY KEY constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    /// Each entry is the set of columns covered by one UNIQUE or PRIMARY KEY constraint. Multi-column
    /// constraints (e.g. `credit (track, artist)`) appear as multi-element inner vectors.
    pub unique_constraints: Vec<Vec<String>>,
}

impl Table {
    /// Looks up a column by name.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// The introspected structure of the database: its tables and its inferred links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub tables: Vec<Table>,
    pub links: Vec<Link>,
}

impl Schema {
    /// Parses an introspection JSON document (as produced by [`introspection_sql`]) into a [`Schema`],
    /// inferring links by convention. The document's own `links` array is ignored and recomputed.
    ///
    /// # Errors
    ///
    /// Returns an error string if the JSON does not match the expected introspection shape.
    pub fn parse(introspection_json: &str) -> Result<Self, String> {
        let raw: RawSchema = serde_json::from_str(introspection_json).map_err(|e| e.to_string())?;
        let links = infer_links(&column_view(&raw.tables));
        let tables = raw
            .tables
            .into_iter()
            .map(|t| Table {
                name: t.name,
                columns: t
                    .columns
                    .into_iter()
                    .map(|c| Column {
                        name: c.name,
                        type_name: c.type_name,
                        nullable: c.nullable,
                    })
                    .collect(),
                unique_constraints: t.unique_constraints,
            })
            .collect();
        Ok(Schema { tables, links })
    }

    /// Looks up a table by name.
    #[must_use]
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// The links whose target is `table` — i.e. every `(from_table, from_column)` in the database
    /// that references this table. Used by the DML delete path to find dangling references.
    pub fn incoming_links<'a>(&'a self, table: &'a str) -> impl Iterator<Item = &'a Link> {
        self.links.iter().filter(move |l| l.to_table == table)
    }

    /// The link whose source is `table.column`, if that column is an inferred foreign key.
    #[must_use]
    pub fn link(&self, table: &str, column: &str) -> Option<&Link> {
        self.links
            .iter()
            .find(|l| l.from_table == table && l.from_column == column)
    }
}

/// Adds convention-inferred table links to a raw introspection JSON document.
///
/// Takes the JSON produced by running [`introspection_sql`] and returns it with its `links` array
/// replaced by the column-name/type convention described in the module docs. Every other part of the
/// document is preserved verbatim, so it stays compatible with whatever shape the Querydown compiler
/// expects. Each emitted link carries `"unique": false`, matching Querydown's `PrimitiveLink`.
///
/// # Errors
///
/// Returns an error string if the JSON has no `tables` array or cannot be re-serialized.
pub fn add_inferred_links(introspection_json: &str) -> Result<String, String> {
    let mut root: Value = serde_json::from_str(introspection_json).map_err(|e| e.to_string())?;
    let raw: RawSchema = serde_json::from_value(root.clone())
        .map_err(|e| format!("Introspected schema JSON did not match the expected shape: {e}"))?;
    let links = infer_links(&column_view(&raw.tables));
    let links_json: Vec<Value> = links
        .into_iter()
        .map(|l| {
            json!({
                "from": { "table": l.from_table, "column": l.from_column },
                "to": { "table": l.to_table, "column": l.to_column },
                "unique": false,
            })
        })
        .collect();
    root["links"] = Value::Array(links_json);
    serde_json::to_string(&root).map_err(|e| e.to_string())
}

/// A minimal `(table_name, [(column_name, column_type)])` view used to infer links.
fn column_view(tables: &[RawTable]) -> Vec<(String, Vec<(String, String)>)> {
    tables
        .iter()
        .map(|t| {
            let cols = t
                .columns
                .iter()
                .map(|c| (c.name.clone(), c.type_name.clone().unwrap_or_default()))
                .collect();
            (t.name.clone(), cols)
        })
        .collect()
}

/// The single source of truth for `RadioCrate`'s foreign-key convention: a `UUID` column whose name
/// matches another table's name links to that table's `id` column.
fn infer_links(tables: &[(String, Vec<(String, String)>)]) -> Vec<Link> {
    let table_names: HashSet<&str> = tables.iter().map(|(name, _)| name.as_str()).collect();
    let mut links = Vec::new();
    for (table_name, columns) in tables {
        for (column_name, column_type) in columns {
            if column_type.eq_ignore_ascii_case("UUID")
                && table_names.contains(column_name.as_str())
            {
                links.push(Link {
                    from_table: table_name.clone(),
                    from_column: column_name.clone(),
                    to_table: column_name.clone(),
                    to_column: "id".to_string(),
                });
            }
        }
    }
    links
}

#[derive(Deserialize)]
struct RawSchema {
    tables: Vec<RawTable>,
}

#[derive(Deserialize)]
struct RawTable {
    name: String,
    columns: Vec<RawColumn>,
    #[serde(default)]
    unique_constraints: Vec<Vec<String>>,
}

#[derive(Deserialize)]
struct RawColumn {
    name: String,
    #[serde(rename = "type", default)]
    type_name: Option<String>,
    #[serde(default)]
    nullable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "tables": [
            { "name": "track", "unique_constraints": [["id"]], "columns": [
                { "name": "id", "type": "UUID", "nullable": false },
                { "name": "title", "type": "VARCHAR", "nullable": true },
                { "name": "album", "type": "UUID", "nullable": true }
            ] },
            { "name": "album", "unique_constraints": [["id"]], "columns": [
                { "name": "id", "type": "UUID", "nullable": false }
            ] },
            { "name": "credit", "unique_constraints": [["track", "artist"]], "columns": [
                { "name": "track", "type": "UUID", "nullable": false },
                { "name": "artist", "type": "UUID", "nullable": false },
                { "name": "order", "type": "FLOAT", "nullable": true }
            ] }
        ],
        "links": []
    }"#;

    #[test]
    fn infers_link_from_uuid_column_matching_table_name() {
        let out: Value = serde_json::from_str(&add_inferred_links(SAMPLE).unwrap()).unwrap();
        let links = out["links"].as_array().unwrap();
        // track.album -> album.id, credit.track -> track.id (credit.artist has no `artist` table here)
        assert_eq!(links.len(), 2);
        assert!(links.iter().any(|l| l["from"]["table"] == "track"
            && l["from"]["column"] == "album"
            && l["to"]["table"] == "album"
            && l["to"]["column"] == "id"
            && l["unique"] == false));
        assert!(links.iter().any(|l| l["from"]["table"] == "credit"
            && l["from"]["column"] == "track"
            && l["to"]["table"] == "track"));
    }

    #[test]
    fn ignores_non_uuid_and_non_matching_columns() {
        let input = r#"{ "tables": [
            { "name": "track", "columns": [
                { "name": "id", "type": "UUID" },
                { "name": "title", "type": "VARCHAR" },
                { "name": "track", "type": "VARCHAR" }
            ] }
        ], "links": [] }"#;
        let out: Value = serde_json::from_str(&add_inferred_links(input).unwrap()).unwrap();
        assert!(out["links"].as_array().unwrap().is_empty());
    }

    #[test]
    fn add_inferred_links_preserves_other_fields_and_overwrites_links() {
        let input = r#"{ "tables": [], "links": [{ "stale": true }], "extra": 1 }"#;
        let out: Value = serde_json::from_str(&add_inferred_links(input).unwrap()).unwrap();
        assert!(out["links"].as_array().unwrap().is_empty());
        assert_eq!(out["extra"], 1);
    }

    #[test]
    fn rejects_json_without_tables() {
        assert!(add_inferred_links(r#"{ "nope": 1 }"#).is_err());
        assert!(Schema::parse(r#"{ "nope": 1 }"#).is_err());
    }

    #[test]
    fn parse_builds_structured_schema_with_links() {
        let schema = Schema::parse(SAMPLE).unwrap();
        let track = schema.table("track").unwrap();
        assert!(!track.column("id").unwrap().nullable);
        assert!(track.column("title").unwrap().nullable);
        assert_eq!(track.unique_constraints, vec![vec!["id".to_string()]]);

        let credit = schema.table("credit").unwrap();
        assert_eq!(
            credit.unique_constraints,
            vec![vec!["track".to_string(), "artist".to_string()]]
        );

        // track.album -> album.id is a link; album is referenced by track.
        assert!(schema.link("track", "album").is_some());
        let incoming: Vec<_> = schema.incoming_links("album").collect();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].from_table, "track");
    }
}
