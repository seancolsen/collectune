//! Compiles a user-authored Querydown query into `DuckDB` SQL.
//!
//! Querydown emits `PostgreSQL`, so we transpile its output to `DuckDB` with
//! polyglot-sql before handing the SQL off to the query API.

use polyglot_sql::{DialectType, transpile};
use querydown::{Compiler, IdentifierResolution, Options, Postgres};

use crate::columns::ColumnMetadata;

/// A compiled Querydown query: the `DuckDB` SQL to run, plus the resolved display
/// metadata for each result column (positionally aligned with the result's columns).
pub(crate) struct CompiledQuery {
    pub(crate) sql: String,
    pub(crate) columns: Vec<ColumnMetadata>,
}

/// Compiles a Querydown query against the given schema JSON into `DuckDB` SQL and its
/// per-column display metadata.
pub(crate) fn querydown_to_duckdb(input: &str, schema_json: &str) -> Result<CompiledQuery, String> {
    let options = Options {
        dialect: Box::new(Postgres()),
        identifier_resolution: IdentifierResolution::Flexible,
    };
    let compiler = Compiler::new(schema_json, options)?;
    let result = compiler.compile(input.to_string())?;
    let columns = result
        .column_metadata
        .iter()
        .map(|meta| ColumnMetadata::from_meta(meta.as_ref()))
        .collect();
    let statements = transpile(&result.sql, DialectType::PostgreSQL, DialectType::DuckDB)
        .map_err(|e| e.to_string())?;
    Ok(CompiledQuery {
        sql: statements.join("\n"),
        columns,
    })
}
