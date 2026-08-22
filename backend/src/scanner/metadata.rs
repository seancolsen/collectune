use std::path::Path;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, Tag, Value};
use symphonia::core::probe::{Hint, ProbeResult};

use super::types::{TrackArtistMetadata, TrackMetadata};

/// Longest duration we are willing to believe a single audio file reports. Comfortably above any
/// real track, live set, or audiobook chapter, and far below the magnitudes that corrupt container
/// metadata produces.
const MAX_PLAUSIBLE_DURATION_SECS: f64 = 24.0 * 60.0 * 60.0;

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
        // Each genre tag on the file becomes its own tag record, so a file
        // carrying several of them ends up linked to several. Trimming can
        // collide two tags that `append_string_value` saw as distinct, hence the
        // second dedup; a blank tag is dropped rather than becoming an
        // empty-named tag shared across every sloppily tagged file.
        genres: genre_values.into_iter().fold(Vec::new(), |mut acc, genre| {
            let genre = genre.trim();
            if !genre.is_empty() && !acc.iter().any(|g| g == genre) {
                acc.push(genre.to_string());
            }
            acc
        }),
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

    // `enable_gapless` makes symphonia derive an Ogg stream's frame count straight from the final
    // page's absolute granule position, rather than from that position plus an "end delay" measured
    // against the preceding page. Some Ogg Vorbis files carry a run of placeholder pages before the
    // end-of-stream page whose granule position is -1 ("no packet completes here"). Symphonia maps
    // that sentinel to a timestamp of u64::MAX, so the end delay it infers is astronomical and the
    // frame count saturates at u64::MAX — yielding a duration of ~418 trillion seconds. Skipping
    // the end delay both dodges that and reports the true playable length.
    let fmt_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };

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

    // Gapless decoding does not rescue a file whose end-of-stream page is itself the one carrying
    // the -1 granule, and other container formats have their own ways of reporting nonsense. Treat
    // an implausible duration as undetermined: a bogus value would otherwise be stored as a
    // duration no human could have produced, and one large enough overflows the INTERVAL that
    // `to_seconds` widens it into, failing the whole scan batch.
    // `contains` also rejects NaN and infinity, both of which compare false against any bound.
    let duration_secs = match duration_secs {
        Some(secs) if (0.0..=MAX_PLAUSIBLE_DURATION_SECS).contains(&secs) => secs,
        Some(secs) => {
            eprintln!(
                "Warning: implausible duration ({secs}s) for {}, recording as unknown",
                file_path.display()
            );
            0.0
        }
        None => 0.0,
    };

    Some((probed, duration_secs))
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
