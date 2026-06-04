//! Compiles a user-authored Querydown query into DuckDB SQL.
//!
//! Querydown emits PostgreSQL, so we transpile its output to DuckDB with
//! polyglot-sql before handing the SQL off to the query API.

use polyglot_sql::{DialectType, transpile};
use querydown::{Compiler, IdentifierResolution, Options, Postgres};

/// Compiles a Querydown query against the given schema JSON into DuckDB SQL.
pub(crate) fn querydown_to_duckdb(input: &str, schema_json: &str) -> Result<String, String> {
    let options = Options {
        dialect: Box::new(Postgres()),
        identifier_resolution: IdentifierResolution::Flexible,
    };
    let compiler = Compiler::new(schema_json, options)?;
    let postgres_sql = compiler.compile(input.to_string())?.sql;
    let statements = transpile(&postgres_sql, DialectType::PostgreSQL, DialectType::DuckDB)
        .map_err(|e| e.to_string())?;
    Ok(statements.join("\n"))
}
