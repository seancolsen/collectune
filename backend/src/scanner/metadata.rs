use std::path::Path;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, Tag, Value};
use symphonia::core::probe::{Hint, ProbeResult};

use super::types::{TrackArtistMetadata, TrackMetadata};

pub fn extension_to_format(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "aac" => Some("aac"),
        "aif" | "aiff" => Some("aiff"),
        "alac" => Some("alac"),
        "ape" => Some("ape"),
        "flac" => Some("flac"),
        "m4a" => Some("mp4"),
        "mp3" => Some("mp3"),
        "ogg" => Some("ogg"),
        "opus" => Some("opus"),
        "wav" => Some("wav"),
        "wma" => Some("wma"),
        "wv" => Some("wv"),
        _ => None,
    }
}

fn parse_tag_value_into_u8(value: &Value) -> Option<u8> {
    match value {
        Value::Binary(_) | Value::Boolean(_) | Value::Flag => None,
        Value::Float(v) => u8::try_from(*v as i64).ok(),
        Value::SignedInt(v) => u8::try_from(*v).ok(),
        Value::UnsignedInt(v) => u8::try_from(*v).ok(),
        Value::String(v) => {
            let start = v.find(|c: char| c.is_ascii_digit())?;
            let end = v[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map_or(v.len(), |i| start + i);
            v[start..end].parse::<u8>().ok()
        }
    }
}

fn parse_tag_value_into_year(value: &Value) -> Option<u16> {
    let current_year = jiff::Zoned::now().year() as u16;

    let year = match value {
        Value::Binary(_) | Value::Boolean(_) | Value::Flag => None,
        Value::Float(v) => u16::try_from(*v as i64).ok(),
        Value::SignedInt(v) => u16::try_from(*v).ok(),
        Value::UnsignedInt(v) => u16::try_from(*v).ok(),
        Value::String(v) => {
            let start = v.find(|c: char| c.is_ascii_digit())?;
            let end = (start + 4).min(v.len());
            v[start..end].parse::<u16>().ok()
        }
    }?;

    (year > 1860 && year <= current_year + 1).then_some(year)
}

fn assemble_tags_into_metadata<'a, T: IntoIterator<Item = &'a Tag>>(tags: T) -> TrackMetadata {
    let mut artist_values = Vec::<String>::new();
    let mut title_values = Vec::<String>::new();
    let mut album_values = Vec::<String>::new();
    let mut genre_values = Vec::<String>::new();

    let append_string_value = |value: &Value, container: &mut Vec<String>| {
        if let Value::String(v) = value
            && !container.contains(v)
        {
            container.push(v.clone());
        }
    };

    let mut date_value: Option<u16> = None;
    let mut track_number_value: Option<u8> = None;
    let mut disk_number_value: Option<u8> = None;

    for tag in tags {
        let Some(key) = tag.std_key else { continue };
        match key {
            StandardTagKey::Artist => append_string_value(&tag.value, &mut artist_values),
            StandardTagKey::TrackTitle => append_string_value(&tag.value, &mut title_values),
            StandardTagKey::Album => append_string_value(&tag.value, &mut album_values),
            StandardTagKey::Genre => append_string_value(&tag.value, &mut genre_values),

            StandardTagKey::Date => {
                date_value = date_value.or_else(|| parse_tag_value_into_year(&tag.value));
            }
            StandardTagKey::TrackNumber => {
                track_number_value =
                    track_number_value.or_else(|| parse_tag_value_into_u8(&tag.value));
            }
            StandardTagKey::DiscNumber => {
                disk_number_value =
                    disk_number_value.or_else(|| parse_tag_value_into_u8(&tag.value));
            }
            _ => {}
        }
    }
    TrackMetadata {
        title: title_values.join(", "),
        track_number: track_number_value,
        disc_number: disk_number_value,
        genre: genre_values.join(", "),
        album: album_values.join(", "),
        year: date_value,
        artists: artist_values
            .into_iter()
            .map(|artist| TrackArtistMetadata { artist, role: None })
            .collect(),
    }
}

fn probe_file(file_path: &Path) -> Option<(ProbeResult, f64)> {
    let file = std::fs::File::open(file_path).ok()?;
    let mss = MediaSourceStream::new(
        Box::new(file),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );

    let mut hint = Hint::new();
    if let Some(ext_str) = file_path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext_str);
    }

    let meta_opts = MetadataOptions::default();
    let fmt_opts = FormatOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .ok()?;

    let duration_secs = probed.format.default_track().and_then(|track| {
        let params = &track.codec_params;
        let time_base = params.time_base?;
        let n_frames = params.n_frames?;
        let time = time_base.calc_time(n_frames);
        Some(time.seconds as f64 + time.frac)
    });

    Some((probed, duration_secs.unwrap_or(0.0)))
}

/// Analyze a file to get its duration in seconds. Returns 0.0 if undetermined.
pub fn get_duration(file_path: &Path) -> f64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        probe_file(file_path).map_or(0.0, |(_, d)| d)
    }));
    if let Ok(d) = result {
        d
    } else {
        eprintln!(
            "Warning: panic while probing {}, skipping duration",
            file_path.display()
        );
        0.0
    }
}

/// Extract full track metadata (tags) plus duration from an audio file.
pub fn get_track_metadata(file_path: &Path) -> Option<(TrackMetadata, f64)> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (mut probed, duration) = probe_file(file_path)?;

        // Read a few packets to ensure metadata is fully loaded (especially for FLAC)
        let mut packets_read = 0;
        while packets_read < 10 {
            match probed.format.next_packet() {
                Ok(_) => packets_read += 1,
                Err(_) => break,
            }
        }

        // ID3v1/ID3v2 tags (e.g. MP3 files)
        let probed_meta = probed.metadata.get();
        let probed_tags = probed_meta
            .as_ref()
            .and_then(|m| m.current())
            .map(symphonia::core::meta::MetadataRevision::tags)
            .unwrap_or_default()
            .iter();

        // Vorbis comments (e.g. FLAC/OGG files)
        let format_meta = probed.format.metadata();
        let format_tags = format_meta
            .current()
            .map(symphonia::core::meta::MetadataRevision::tags)
            .unwrap_or_default()
            .iter();

        let metadata = assemble_tags_into_metadata(probed_tags.chain(format_tags));

        Some((metadata, duration))
    }));

    if let Ok(inner) = result {
        inner
    } else {
        eprintln!(
            "Warning: panic while reading {}, skipping",
            file_path.display()
        );
        None
    }
}
