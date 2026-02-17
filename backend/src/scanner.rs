use std::fs;
use std::path::{Path, PathBuf};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, Tag, Value};
use symphonia::core::probe::Hint;

static AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "m4a", "opus", "wma", "aac", "aiff", "aif", "alac", "ape", "wav", "wv",
];

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn get_audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    files.extend(get_audio_files(&path));
                } else if path.is_file() && is_audio_file(&path) {
                    files.push(path);
                }
            }
        }
    }
    files
}

#[derive(Debug)]
struct TrackMetadata {
    title: String,
    track_number: Option<u8>,
    disc_number: Option<u8>,
    genre: String,
    album: String,
    year: Option<u16>,
    artists: Vec<TrackArtistMetadata>,
}

#[derive(Debug)]
struct TrackArtistMetadata {
    artist: String,
    role: Option<String>,
}

fn parse_tag_value_into_u8(value: &Value) -> Option<u8> {
    match value {
        Value::Binary(_) => None,
        Value::Boolean(_) => None,
        Value::Flag => None,
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
        Value::Binary(_) => None,
        Value::Boolean(_) => None,
        Value::Flag => None,
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

    let append_string_value = |value: &Value, container: &mut Vec<String>| match value {
        Value::String(v) => {
            if !container.contains(v) {
                container.push(v.clone());
            }
        }
        _ => (),
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
                date_value = date_value.or_else(|| parse_tag_value_into_year(&tag.value))
            }
            StandardTagKey::TrackNumber => {
                track_number_value =
                    track_number_value.or_else(|| parse_tag_value_into_u8(&tag.value))
            }
            StandardTagKey::DiscNumber => {
                disk_number_value =
                    disk_number_value.or_else(|| parse_tag_value_into_u8(&tag.value))
            }
            _ => continue,
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

fn get_track_metadata(file_path: &PathBuf) -> Option<TrackMetadata> {
    let file = std::fs::File::open(&file_path).ok()?;

    // Guard against panics in symphonia, just to be safe and avoid crashing. For example, in my
    // testing I observed that symphonia seems panic when it hit a .mood file.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // Create a hint to help the format registry guess the format
        let mut hint = Hint::new();
        if let Some(extension) = file_path.extension() {
            if let Some(ext_str) = extension.to_str() {
                hint.with_extension(ext_str);
            }
        }

        // Probe the file for metadata
        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        let mut probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .ok()?;

        // Extract metadata - need to read packets to fully populate metadata for some formats like
        // FLAC
        let mut format = probed.format;

        // Read a few packets to ensure metadata is fully loaded (especially for FLAC)
        let mut packets_read = 0;
        while packets_read < 10 {
            match format.next_packet() {
                Ok(_) => packets_read += 1,
                Err(_) => break,
            }
        }

        // e.g. for ID3v1/ID3v2 tags in MP3 files
        let probed_meta = probed.metadata.get();
        let probed_tags = probed_meta
            .as_ref()
            .and_then(|m| m.current())
            .map(|r| r.tags())
            .unwrap_or_default()
            .iter();

        // e.g. for Vorbis comments in FLAC/OGG
        let format_meta = format.metadata();
        let format_tags = format_meta
            .current()
            .map(|r| r.tags())
            .unwrap_or_default()
            .iter();

        let metadata = assemble_tags_into_metadata(probed_tags.chain(format_tags));

        Some(metadata)
    }));

    match result {
        Ok(metadata) => metadata,
        Err(_) => {
            eprintln!("Warning: panic while reading {:?}, skipping", file_path);
            None
        }
    }
}

pub fn scan(collection_path: &Path) {
    for file_path in get_audio_files(collection_path) {
        if let Some(metadata) = get_track_metadata(&file_path) {
            println!("{:?}", file_path);
            println!("{:?}", metadata);
        }
    }
}
