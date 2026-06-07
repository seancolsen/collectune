//! Per-column display metadata.
//!
//! Querydown lets the user attach arbitrary metadata to each result column in their
//! query source. The compiler surfaces this as one [`MetaValue`] (or `None`) per output
//! column. Here we normalize each blob into a [`ColumnMetadata`] with every field
//! resolved to a concrete value, so rendering never has to think about defaults.

use querydown::MetaValue;
use serde::Deserialize;

use crate::format::Formatter;

/// Default column min width, in pixels, when none is specified.
const DEFAULT_MIN_WIDTH: f32 = 0.0;
/// Default column max width, in pixels, when none is specified.
const DEFAULT_MAX_WIDTH: f32 = 500.0;

/// Whether a column's text uses the normal or a de-emphasized ("light") color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FontColor {
    #[default]
    Default,
    Light,
}

/// Column text size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FontSize {
    #[default]
    Normal,
    Small,
}

/// Horizontal alignment of a column's text within its cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TextAlign {
    #[default]
    Left,
    Right,
    Center,
}

/// Fully-resolved display metadata for a single result column. Every field has a
/// concrete value (defaults already applied), so rendering can use it directly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColumnMetadata {
    pub(crate) hide: bool,
    pub(crate) min_width: f32,
    pub(crate) max_width: f32,
    pub(crate) font_color: FontColor,
    pub(crate) font_size: FontSize,
    pub(crate) text_align: TextAlign,
    pub(crate) prefix: String,
    pub(crate) suffix: String,
    pub(crate) formatter: Option<Formatter>,
}

impl Default for ColumnMetadata {
    fn default() -> Self {
        Self {
            hide: false,
            min_width: DEFAULT_MIN_WIDTH,
            max_width: DEFAULT_MAX_WIDTH,
            font_color: FontColor::default(),
            font_size: FontSize::default(),
            text_align: TextAlign::default(),
            prefix: String::new(),
            suffix: String::new(),
            formatter: None,
        }
    }
}

/// The user-facing width spec: a plain number, or a one/two-element list.
#[derive(Deserialize)]
#[serde(untagged)]
enum WidthSpec {
    Single(f32),
    List(Vec<f32>),
}

/// The raw metadata blob as written by the user, before defaults are applied. Every
/// field is optional; unknown keys are ignored.
#[derive(Default, Deserialize)]
struct RawColumnMetadata {
    hide: Option<String>,
    width: Option<WidthSpec>,
    color: Option<String>,
    size: Option<String>,
    align: Option<String>,
    prefix: Option<String>,
    suffix: Option<String>,
    /// Kept as a raw JSON value and parsed separately so a malformed formatter blob
    /// yields `None` instead of discarding the whole column's metadata.
    formatter: Option<serde_json::Value>,
}

/// Resolves a [`WidthSpec`] into a `(min_width, max_width)` pair, applying the rules:
/// a plain number sets both bounds equal; `[min]` keeps the default max; `[min, max]`
/// sets both (extra elements ignored); an empty list falls back to the defaults. If the
/// resulting min exceeds the max, the two are swapped.
fn resolve_width(spec: Option<WidthSpec>) -> (f32, f32) {
    let (mut min, mut max) = match spec {
        Some(WidthSpec::Single(n)) => (n, n),
        Some(WidthSpec::List(list)) => match list.as_slice() {
            [] => (DEFAULT_MIN_WIDTH, DEFAULT_MAX_WIDTH),
            [min] => (*min, DEFAULT_MAX_WIDTH),
            [min, max, ..] => (*min, *max),
        },
        None => (DEFAULT_MIN_WIDTH, DEFAULT_MAX_WIDTH),
    };
    if min > max {
        std::mem::swap(&mut min, &mut max);
    }
    (min, max)
}

fn resolve_font_color(value: Option<&str>) -> FontColor {
    match value {
        Some("light") => FontColor::Light,
        _ => FontColor::Default,
    }
}

fn resolve_font_size(value: Option<&str>) -> FontSize {
    match value {
        Some("small") => FontSize::Small,
        _ => FontSize::Normal,
    }
}

fn resolve_text_align(value: Option<&str>) -> TextAlign {
    match value {
        Some("right") => TextAlign::Right,
        Some("center") => TextAlign::Center,
        _ => TextAlign::Left,
    }
}

impl ColumnMetadata {
    /// Normalizes one column's metadata blob (or its absence) into resolved settings. A
    /// `None` blob, or any blob that isn't a JSON object, yields all defaults.
    pub(crate) fn from_meta(meta: Option<&MetaValue>) -> Self {
        let raw = meta
            .and_then(|m| serde_json::to_value(m).ok())
            .and_then(|v| serde_json::from_value::<RawColumnMetadata>(v).ok())
            .unwrap_or_default();
        let (min_width, max_width) = resolve_width(raw.width);
        Self {
            hide: raw.hide.as_deref() == Some("yes"),
            min_width,
            max_width,
            font_color: resolve_font_color(raw.color.as_deref()),
            font_size: resolve_font_size(raw.size.as_deref()),
            text_align: resolve_text_align(raw.align.as_deref()),
            prefix: raw.prefix.unwrap_or_default(),
            suffix: raw.suffix.unwrap_or_default(),
            formatter: raw
                .formatter
                .and_then(|v| serde_json::from_value::<Formatter>(v).ok()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_yields_defaults() {
        assert_eq!(ColumnMetadata::from_meta(None), ColumnMetadata::default());
    }

    #[test]
    fn non_object_yields_defaults() {
        let meta = MetaValue::String("nonsense".to_string());
        assert_eq!(
            ColumnMetadata::from_meta(Some(&meta)),
            ColumnMetadata::default()
        );
    }

    fn object(entries: Vec<(&str, MetaValue)>) -> MetaValue {
        MetaValue::Object(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    fn num(n: &str) -> MetaValue {
        MetaValue::Number(n.to_string())
    }

    #[test]
    fn parses_all_fields() {
        let meta = object(vec![
            ("hide", MetaValue::String("yes".to_string())),
            ("color", MetaValue::String("light".to_string())),
            ("size", MetaValue::String("small".to_string())),
            ("align", MetaValue::String("center".to_string())),
            ("prefix", MetaValue::String("$".to_string())),
            ("suffix", MetaValue::String(" kg".to_string())),
        ]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert!(m.hide);
        assert_eq!(m.font_color, FontColor::Light);
        assert_eq!(m.font_size, FontSize::Small);
        assert_eq!(m.text_align, TextAlign::Center);
        assert_eq!(m.prefix, "$");
        assert_eq!(m.suffix, " kg");
    }

    #[test]
    fn hide_only_yes_hides() {
        let meta = object(vec![("hide", MetaValue::String("no".to_string()))]);
        assert!(!ColumnMetadata::from_meta(Some(&meta)).hide);
    }

    #[test]
    fn unknown_enum_values_fall_back_to_defaults() {
        let meta = object(vec![
            ("color", MetaValue::String("rainbow".to_string())),
            ("align", MetaValue::String("justify".to_string())),
        ]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!(m.font_color, FontColor::Default);
        assert_eq!(m.text_align, TextAlign::Left);
    }

    #[test]
    fn width_plain_number_sets_both_bounds() {
        let meta = object(vec![("width", num("120"))]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!((m.min_width, m.max_width), (120.0, 120.0));
    }

    #[test]
    fn width_single_element_list_keeps_default_max() {
        let meta = object(vec![("width", MetaValue::Array(vec![num("80")]))]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!((m.min_width, m.max_width), (80.0, DEFAULT_MAX_WIDTH));
    }

    #[test]
    fn width_two_element_list_sets_both() {
        let meta = object(vec![(
            "width",
            MetaValue::Array(vec![num("50"), num("300")]),
        )]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!((m.min_width, m.max_width), (50.0, 300.0));
    }

    #[test]
    fn width_swaps_when_min_exceeds_max() {
        let meta = object(vec![(
            "width",
            MetaValue::Array(vec![num("300"), num("50")]),
        )]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!((m.min_width, m.max_width), (50.0, 300.0));
    }

    #[test]
    fn parses_formatter_blob() {
        let meta = object(vec![(
            "formatter",
            object(vec![("type", MetaValue::String("duration".to_string()))]),
        )]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!(m.formatter, Some(Formatter::Duration {}));
    }

    #[test]
    fn unknown_formatter_type_is_ignored_without_disturbing_other_fields() {
        let meta = object(vec![
            ("align", MetaValue::String("right".to_string())),
            (
                "formatter",
                object(vec![("type", MetaValue::String("bogus".to_string()))]),
            ),
        ]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!(m.formatter, None);
        assert_eq!(m.text_align, TextAlign::Right);
    }

    #[test]
    fn width_empty_list_uses_defaults() {
        let meta = object(vec![("width", MetaValue::Array(vec![]))]);
        let m = ColumnMetadata::from_meta(Some(&meta));
        assert_eq!(
            (m.min_width, m.max_width),
            (DEFAULT_MIN_WIDTH, DEFAULT_MAX_WIDTH)
        );
    }
}
