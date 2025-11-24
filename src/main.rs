use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Parser)]
#[command(name = "collectune")]
#[command(about = "A tool for managing audio file collections")]
struct Args {
    /// Path to the collection of audio files
    collection_path: String,
}

fn main() {
    let args = Args::parse();

    let path = Path::new(&args.collection_path);
    
    if !path.exists() {
        eprintln!("Error: The path '{}' does not exist.", args.collection_path);
        std::process::exit(1);
    }
    
    if !path.is_dir() {
        eprintln!("Error: The path '{}' is not a directory.", args.collection_path);
        std::process::exit(1);
    }

    // Scan the directory for files
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let file_path = entry.path();
                if file_path.is_file() {
                    files.push(file_path);
                }
            }
        }
    }

    if files.is_empty() {
        eprintln!("Error: No files found in the collection directory.");
        std::process::exit(1);
    }

    // Sort files to get consistent first file
    files.sort();
    let first_file = &files[0];

    // Open the audio file
    let file = match std::fs::File::open(first_file) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: Failed to open file '{}': {}", first_file.display(), e);
            std::process::exit(1);
        }
    };

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    
    // Create a hint to help the format registry guess the format
    let mut hint = Hint::new();
    if let Some(extension) = first_file.extension() {
        if let Some(ext_str) = extension.to_str() {
            hint.with_extension(ext_str);
        }
    }

    // Probe the file for metadata
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let mut probed = match symphonia::default::get_probe().format(&hint, mss, &fmt_opts, &meta_opts) {
        Ok(probed) => probed,
        Err(e) => {
            eprintln!("Error: Failed to probe audio file '{}': {}", first_file.display(), e);
            std::process::exit(1);
        }
    };

    // Extract metadata
    let mut artist: Option<String> = None;
    let mut title: Option<String> = None;

    if let Some(metadata) = probed.metadata.get() {
        if let Some(metadata_rev) = metadata.current() {
            for tag in metadata_rev.tags() {
                match tag.std_key {
                    Some(symphonia::core::meta::StandardTagKey::Artist) => {
                        artist = Some(tag.value.to_string());
                    }
                    Some(symphonia::core::meta::StandardTagKey::TrackTitle) => {
                        title = Some(tag.value.to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    // Print the results
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
