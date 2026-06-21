//! Compiles a user-authored Querydown query into `DuckDB` SQL.
//!
//! Querydown emits `DuckDB` SQL directly via its `DuckDB` dialect, so the SQL it
//! produces is handed straight off to the query API.

use querydown::ast::{Definitions, Query};
use querydown::{
    Compiler, DuckDB, IdentifierResolution, Options, parse, parse_conditions, parse_display,
    parse_sorting,
};

use crate::columns::ColumnMetadata;
use crate::query_def::{CompileSource, QuerySections};

/// A compiled Querydown query: the `DuckDB` SQL to run, plus the resolved display
/// metadata for each result column (positionally aligned with the result's columns).
pub(crate) struct CompiledQuery {
    pub(crate) sql: String,
    pub(crate) columns: Vec<ColumnMetadata>,
}

/// Compiles a query — given as its resolved [`CompileSource`] — against the schema
/// JSON into `DuckDB` SQL and its per-column display metadata.
///
/// In sectioned mode each section is parsed with its own section parser, so the
/// filter, sort, and display inputs can't borrow each other's syntax (e.g. a stray
/// `$` result column at the end of the sort input is rejected as a sort error rather
/// than silently changing the result set). The parsed sections are then reassembled
/// into a single [`Query`]. In full-querydown mode the entire query is parsed in one
/// pass with the whole-query parser. Either way the resulting [`Query`] is compiled.
/// Collectune has no UI for the definitions section, so it is always empty.
pub(crate) fn querydown_to_duckdb(
    source: &CompileSource,
    schema_json: &str,
) -> Result<CompiledQuery, String> {
    let query = match source {
        CompileSource::Sections(sections) => query_from_sections(sections)?,
        CompileSource::Full(text) => parse(text).map_err(|e| format!("Querydown: {e}"))?,
    };
    let options = Options {
        dialect: Box::new(DuckDB()),
        identifier_resolution: IdentifierResolution::Flexible,
    };
    let compiler = Compiler::new(schema_json, options)?;
    let result = compiler.compile_query(query)?;
    let columns = result
        .column_annotations
        .iter()
        .map(|meta| ColumnMetadata::from_meta(meta.as_ref()))
        .collect();
    Ok(CompiledQuery {
        sql: result.sql,
        columns,
    })
}

/// Parses the per-section Querydown source (each with its own section parser) and
/// reassembles it into a single [`Query`].
fn query_from_sections(sections: &QuerySections) -> Result<Query, String> {
    let conditions =
        parse_conditions(&sections.filter).map_err(|e| format!("Filter section: {e}"))?;
    let sorting = parse_sorting(&sections.sort).map_err(|e| format!("Sort section: {e}"))?;
    let display = parse_display(&sections.display).map_err(|e| format!("Display section: {e}"))?;
    Ok(Query::from_parts(
        sections.base.clone(),
        Definitions::default(),
        conditions,
        sorting,
        display,
    ))
}
