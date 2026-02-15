use std::fs;
use std::path::{Path, PathBuf};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn scan_directory_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    // Recursively scan subdirectories
                    files.extend(scan_directory_recursive(&path));
                } else if path.is_file() {
                    files.push(path);
                }
            }
        }
    }

    files
}

fn process_audio_file(file_path: &Path) -> Option<(Option<String>, Option<String>)> {
    // Open the audio file
    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return None,
    };

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

    let probed = match symphonia::default::get_probe().format(&hint, mss, &fmt_opts, &meta_opts) {
        Ok(probed) => probed,
        Err(_) => return None, // Not a recognized audio file
    };

    // Extract metadata - need to read packets to fully populate metadata for some formats like FLAC
    let mut format = probed.format;

    // Read a few packets to ensure metadata is fully loaded (especially for FLAC)
    let mut packets_read = 0;
    while packets_read < 10 {
        match format.next_packet() {
            Ok(_) => packets_read += 1,
            Err(_) => break,
        }
    }

    // Now extract metadata from the format
    let mut artist: Option<String> = None;
    let mut title: Option<String> = None;

    let metadata = format.metadata();
    if let Some(metadata_rev) = metadata.current() {
        for tag in metadata_rev.tags() {
            // Check visual tag keys first (case-insensitive) - FLAC uses Vorbis comments
            let tag_key_lower = tag.key.to_lowercase();
            if artist.is_none() && (tag_key_lower == "artist" || tag_key_lower == "album artist") {
                artist = Some(tag.value.to_string());
            }
            if title.is_none() && (tag_key_lower == "title" || tag_key_lower == "tracktitle") {
                title = Some(tag.value.to_string());
            }

            // Also check standard keys
            match tag.std_key {
                Some(symphonia::core::meta::StandardTagKey::Artist) => {
                    if artist.is_none() {
                        artist = Some(tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::TrackTitle) => {
                    if title.is_none() {
                        title = Some(tag.value.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    Some((artist, title))
}

pub fn scan(collection_path: &Path) {
    let files = scan_directory_recursive(collection_path);

    for file_path in files {
        if let Some((artist, title)) = process_audio_file(&file_path) {
            if let Some(artist_name) = artist {
                println!("Artist: {}", artist_name);
            } else {
                println!("Artist: (not found)");
            }

            if let Some(track_title) = title {
                println!("Title: {}", track_title);
            } else {
                println!("Title: (not found)");
            }
        }
    }
}
