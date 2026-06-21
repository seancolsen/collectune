//! Compiles a user-authored Querydown query into `DuckDB` SQL.
//!
//! Querydown emits `DuckDB` SQL directly via its `DuckDB` dialect, so the SQL it
//! produces is handed straight off to the query API.

use querydown::ast::Query;
use querydown::{
    Compiler, DuckDB, IdentifierResolution, Options, parse, parse_conditions, parse_definitions,
    parse_display, parse_sorting,
};

use crate::columns::ColumnMetadata;
use crate::query_def::{CompileSource, QuerySections};

/// Querydown definitions prepended to every query before compilation. These set
/// up Collectune's computed columns (e.g. `#track.year`) and custom comparisons
/// (e.g. the default text search and `#track.artist`) so that queries — whether
/// hand-written or composed from sections — can refer to them. For now this is
/// hard-coded here in the frontend.
///
/// In full-querydown mode it is concatenated ahead of the user's query (newline
/// separated); in sectioned mode it is parsed with [`parse_definitions`] and fed
/// into the reassembled [`Query`] as its definitions section.
const PRELUDE: &str = r"#track.firstplay = #play.timestamp%min
#track.lastplay = #play.timestamp%max
#track.artists = #artist.name%list(\\name)
#track.year = album.year
#track.added = file.added
#track.duration = file.duration
#track.playcount = #play
#track.number = track_number
#track.__querydown_default_text_search:@x = [
  title:@x
  genre:@x
  album.title:@x
  ++#artist{name:@x}
]
#track.artist:@x = ++#artist{name:@x}";

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
/// pass with the whole-query parser. Either way the [`PRELUDE`] definitions are
/// supplied — concatenated ahead of the full query, or fed in as the definitions
/// section of the reassembled query — and the resulting [`Query`] is compiled.
pub(crate) fn querydown_to_duckdb(
    source: &CompileSource,
    schema_json: &str,
) -> Result<CompiledQuery, String> {
    let query = match source {
        CompileSource::Sections(sections) => query_from_sections(sections)?,
        CompileSource::Full(text) => {
            parse(&format!("{PRELUDE}\n{text}")).map_err(|e| format!("Querydown: {e}"))?
        }
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
    let definitions = parse_definitions(PRELUDE).map_err(|e| format!("Prelude: {e}"))?;
    let conditions =
        parse_conditions(&sections.filter).map_err(|e| format!("Filter section: {e}"))?;
    let sorting = parse_sorting(&sections.sort).map_err(|e| format!("Sort section: {e}"))?;
    let display = parse_display(&sections.display).map_err(|e| format!("Display section: {e}"))?;
    Ok(Query::from_parts(
        sections.base.clone(),
        definitions,
        conditions,
        sorting,
        display,
    ))
}
