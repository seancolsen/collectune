//! The structured, four-part query definition: base table, filter, sort, and
//! display (result columns). Each of the last three sections holds raw
//! Querydown fragments and/or references to saved presets, and at run time the
//! definition is resolved into per-section Querydown source ([`QuerySections`]),
//! which the compiler parses one section at a time.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::rpc::Preset;

/// The three query sections that have builder UIs and saveable presets. (The
/// fourth part of a query — the base table — is just a table name.)
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Section {
    Filter,
    Sort,
    Display,
}

impl Section {
    /// The short label used on the toolbar button.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Section::Filter => "Filter",
            Section::Sort => "Sort",
            Section::Display => "Display",
        }
    }

    /// The noun used in builder headings and menus ("Sorting options",
    /// "CUSTOM SORTING", …).
    pub(crate) fn noun(self) -> &'static str {
        match self {
            Section::Filter => "Filter",
            Section::Sort => "Sorting",
            Section::Display => "Display",
        }
    }
}

/// A sort or display section: either hand-written Querydown or a reference to
/// a saved preset. (The filter section instead uses [`FilterParts`], which can
/// combine custom code with several presets.)
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SectionContent {
    Custom(String),
    Preset(Uuid),
}

impl Default for SectionContent {
    fn default() -> Self {
        SectionContent::Custom(String::new())
    }
}

/// The filter section: custom conditions combined (via AND) with any number
/// of presets.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FilterParts {
    pub(crate) custom: String,
    pub(crate) presets: Vec<Uuid>,
}

/// A query split into its four Querydown parts.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QueryDefinition {
    /// The base table name (without the `#` sigil). Empty until chosen.
    pub(crate) base: String,
    pub(crate) filter: FilterParts,
    pub(crate) sort: SectionContent,
    pub(crate) display: SectionContent,
    /// When `Some`, the query is in full-querydown mode: this holds the entire
    /// hand-written query and the sectioned parts above are ignored. `None`
    /// (the default) is the sectioned mode driven by the builder UIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) full: Option<String>,
}

impl QueryDefinition {
    /// Whether this query is in full-querydown mode (a single hand-written query)
    /// rather than the default sectioned mode.
    pub(crate) fn is_full(&self) -> bool {
        self.full.is_some()
    }

    /// Whether there is enough here to compile: in full mode, non-empty query
    /// text; otherwise a base table has been chosen.
    pub(crate) fn is_runnable(&self) -> bool {
        match &self.full {
            Some(full) => !full.trim().is_empty(),
            None => !self.base.trim().is_empty(),
        }
    }

    /// The JSON form persisted in the backend's `query.definition` column.
    pub(crate) fn to_stored(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parses a stored definition: structured JSON for queries saved by this
    /// version, otherwise a best-effort split of a legacy raw-Querydown query.
    pub(crate) fn from_stored(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_else(|_| Self::from_legacy(raw))
    }

    /// Splits a legacy single-textarea Querydown query into the four parts.
    /// The split is positional (base table, then conditions, then `\\` sorting
    /// expressions, then `$` result columns) and doesn't account for `$` or
    /// `\\` inside string literals, which is acceptable for one-time migration
    /// of hand-written queries.
    fn from_legacy(raw: &str) -> Self {
        let text = raw.trim();
        let (base, rest) = match text.strip_prefix('#') {
            Some(stripped) => {
                let end = stripped.find(char::is_whitespace).unwrap_or(stripped.len());
                (stripped[..end].to_string(), &stripped[end..])
            }
            None => (String::new(), text),
        };
        let display_at = rest.find('$');
        let sort_at = rest
            .find("\\\\")
            .filter(|s| display_at.is_none_or(|d| *s < d));
        let filter_end = sort_at.or(display_at).unwrap_or(rest.len());
        let sort = match sort_at {
            Some(s) => rest[s..display_at.unwrap_or(rest.len())].trim(),
            None => "",
        };
        let display = match display_at {
            Some(d) => rest[d..].trim(),
            None => "",
        };
        Self {
            base,
            filter: FilterParts {
                custom: rest[..filter_end].trim().to_string(),
                presets: Vec::new(),
            },
            sort: SectionContent::Custom(sort.to_string()),
            display: SectionContent::Custom(display.to_string()),
            full: None,
        }
    }

    /// Resolves the definition into per-section Querydown source, resolving each
    /// preset reference against the loaded preset list. Unlike a single assembled
    /// query string, the sections are kept apart so the compiler can parse each
    /// one with its own section parser — which keeps filter, sort, and display
    /// syntax from leaking across section boundaries.
    ///
    /// The filter section concatenates the custom conditions with each preset's
    /// fragment (whitespace-separated); top-level conditions combine as AND. The
    /// sort and display sections each resolve to a single fragment.
    pub(crate) fn assemble(&self, presets: &[Preset]) -> Result<CompileSource, String> {
        if let Some(full) = &self.full {
            let full = full.trim();
            if full.is_empty() {
                return Err("The query is empty.".to_string());
            }
            return Ok(CompileSource::Full(full.to_string()));
        }
        let base = self.base.trim();
        if base.is_empty() {
            return Err("No base table selected.".to_string());
        }
        let mut filter_parts: Vec<&str> = Vec::new();
        let custom = self.filter.custom.trim();
        if !custom.is_empty() {
            filter_parts.push(custom);
        }
        for id in &self.filter.presets {
            let fragment = preset_definition(presets, *id)?.trim();
            if !fragment.is_empty() {
                filter_parts.push(fragment);
            }
        }
        Ok(CompileSource::Sections(QuerySections {
            base: base.to_string(),
            filter: filter_parts.join("\n"),
            sort: resolve_section(&self.sort, presets)?,
            display: resolve_section(&self.display, presets)?,
        }))
    }

    /// Converts this definition into a single full-querydown string for the
    /// full-query editor: the base table prefixed with `#`, then the filter,
    /// sort, and display fragments concatenated (newline-separated). Preset
    /// references are resolved against `presets`; a missing preset's fragment is
    /// simply omitted so the conversion always yields editable text. An
    /// already-full definition returns its existing text unchanged.
    pub(crate) fn to_full_query(&self, presets: &[Preset]) -> String {
        if let Some(full) = &self.full {
            return full.clone();
        }
        let mut filter_parts: Vec<String> = Vec::new();
        let custom = self.filter.custom.trim();
        if !custom.is_empty() {
            filter_parts.push(custom.to_string());
        }
        for id in &self.filter.presets {
            if let Ok(fragment) = preset_definition(presets, *id) {
                let fragment = fragment.trim();
                if !fragment.is_empty() {
                    filter_parts.push(fragment.to_string());
                }
            }
        }
        let sort = resolve_section(&self.sort, presets).unwrap_or_default();
        let display = resolve_section(&self.display, presets).unwrap_or_default();
        let mut parts = vec![format!("#{}", self.base.trim())];
        for fragment in [filter_parts.join("\n"), sort, display] {
            let fragment = fragment.trim();
            if !fragment.is_empty() {
                parts.push(fragment.to_string());
            }
        }
        parts.join("\n")
    }
}

/// What [`QueryDefinition::assemble`] resolves a query into for compilation:
/// either the per-section Querydown source (sectioned mode) or a single
/// hand-written query (full-querydown mode).
pub(crate) enum CompileSource {
    Sections(QuerySections),
    Full(String),
}

/// A query's four parts resolved into per-section Querydown source, ready to be
/// parsed section-by-section by the compiler. Preset references have been
/// resolved to their underlying fragments (and, for the filter, concatenated).
pub(crate) struct QuerySections {
    /// The base table name, without the `#` sigil.
    pub(crate) base: String,
    /// Filter conditions: custom code followed by each preset's fragment.
    pub(crate) filter: String,
    /// Standalone `\\` sorting expressions.
    pub(crate) sort: String,
    /// `$`-prefixed result columns.
    pub(crate) display: String,
}

/// Resolves a sort/display section to its Querydown fragment, looking up a
/// preset reference against `presets`.
fn resolve_section(content: &SectionContent, presets: &[Preset]) -> Result<String, String> {
    let text = match content {
        SectionContent::Custom(text) => text.as_str(),
        SectionContent::Preset(id) => preset_definition(presets, *id)?,
    };
    Ok(text.trim().to_string())
}

fn preset_definition(presets: &[Preset], id: Uuid) -> Result<&str, String> {
    presets
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.definition.as_str())
        .ok_or_else(|| "This query references a preset that no longer exists.".to_string())
}

/// Serde codec for the `definition` field of [`crate::rpc::Query`]: on the
/// wire (and in the database) the structured definition travels as a JSON
/// string inside the existing `definition` text column.
pub(crate) mod codec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::QueryDefinition;

    pub(crate) fn serialize<S: Serializer>(
        def: &QueryDefinition,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        def.to_stored().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<QueryDefinition, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(QueryDefinition::from_stored(&raw))
    }
}
