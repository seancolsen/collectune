//! The structured, four-part query definition: base table, filter, sort, and
//! display (result columns). Each of the last three sections holds raw
//! Querydown fragments and/or references to saved presets, and the whole
//! definition is assembled back into a single Querydown query at run time.

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
}

impl QueryDefinition {
    /// Whether there is enough here to compile: a base table has been chosen.
    pub(crate) fn is_runnable(&self) -> bool {
        !self.base.trim().is_empty()
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
        }
    }

    /// Stitches the four parts back into a single Querydown query, resolving
    /// preset references against the loaded preset list. Sections are plain
    /// fragments concatenated in the order the language requires: base table,
    /// conditions (custom then presets — top-level conditions combine as AND),
    /// `\\` sorting expressions, then `$` result columns.
    pub(crate) fn assemble(&self, presets: &[Preset]) -> Result<String, String> {
        let base = self.base.trim();
        if base.is_empty() {
            return Err("No base table selected.".to_string());
        }
        let mut parts = vec![format!("#{base}")];
        let mut push = |fragment: &str| {
            if !fragment.trim().is_empty() {
                parts.push(fragment.trim().to_string());
            }
        };
        push(&self.filter.custom);
        for id in &self.filter.presets {
            push(preset_definition(presets, *id)?);
        }
        for content in [&self.sort, &self.display] {
            match content {
                SectionContent::Custom(text) => push(text),
                SectionContent::Preset(id) => push(preset_definition(presets, *id)?),
            }
        }
        Ok(parts.join("\n"))
    }
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
