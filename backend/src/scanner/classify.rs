use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use uuid::Uuid;

use super::metadata::{extension_to_format, get_duration, get_track_metadata};
use super::types::*;

static AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "m4a", "opus", "wma", "aac", "aiff", "aif", "alac", "ape", "wav", "wv",
];

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

pub fn get_audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(get_audio_files(&path));
            } else if path.is_file() && is_audio_file(&path) {
                files.push(path);
            }
        }
    }
    files
}

fn hash_file(path: &Path) -> Option<[u8; 32]> {
    let data = fs::read(path).ok()?;
    Some(*blake3::hash(&data).as_bytes())
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn classify_file(path: &Path, existing: &ExistingFiles) -> Option<FileClassification> {
    let path_str = path.to_string_lossy().to_string();

    let hash = hash_file(path)?;
    let path_match = existing.by_path.get(&path_str);
    let hash_match = existing.by_hash.get(&hash);

    match (path_match, hash_match) {
        // Both match: file is unchanged
        (Some((_, existing_hash)), _) if *existing_hash == hash => {
            Some(FileClassification::Skipped { path: path_str })
        }

        // Hash matches but path does not (path_match is None or hash differs)
        (None, Some(entries)) => {
            // Find the first entry whose original path no longer exists on disk
            for (id, original_path) in entries {
                if !Path::new(original_path).exists() {
                    return Some(FileClassification::Moved {
                        id: *id,
                        path: path_str,
                    });
                }
            }
            // All original paths still exist: treat as new
            classify_as_new(path_str, hash)
        }

        // Path matches but hash does not: modified
        (Some((id, _)), _) => {
            let size = file_size(path);
            let duration = get_duration(path);
            Some(FileClassification::Modified {
                id: *id,
                path: path_str,
                hash,
                size,
                duration,
            })
        }

        // Neither matches: new
        (None, None) => classify_as_new(path_str, hash),
    }
}

fn classify_as_new(path_str: String, hash: [u8; 32]) -> Option<FileClassification> {
    let path = Path::new(&path_str);
    let ext = path.extension()?.to_str()?;
    let format = extension_to_format(ext)?;

    let (metadata, duration) = get_track_metadata(path)?;
    let size = file_size(path);

    Some(FileClassification::New(NewFileData {
        path: path_str,
        hash,
        size,
        duration,
        format: format.to_string(),
        metadata,
    }))
}

fn aggregate(classifications: Vec<FileClassification>) -> ScanResults {
    let mut skipped = Vec::new();
    let mut moved = Vec::new();
    let mut modified = Vec::new();
    let mut new_files = Vec::new();

    for c in classifications {
        match c {
            FileClassification::Skipped { path } => skipped.push(path),
            FileClassification::Moved { id, path } => moved.push(MovedEntry { id, path }),
            FileClassification::Modified {
                id,
                path,
                hash,
                size,
                duration,
            } => modified.push(ModifiedEntry {
                id,
                path,
                hash,
                size,
                duration,
            }),
            FileClassification::New(data) => new_files.push(data),
        }
    }

    ScanResults {
        skipped,
        moved,
        modified,
        new_files,
    }
}

/// If a file ID appears in both moved and modified, the hash-based match (moved)
/// wins. The path-matched entry is reclassified as new.
pub fn resolve_conflicts(results: &mut ScanResults) {
    let moved_ids: HashSet<Uuid> = results.moved.iter().map(|m| m.id).collect();

    let conflicting: Vec<ModifiedEntry> = results
        .modified
        .extract_if(.., |m| moved_ids.contains(&m.id))
        .collect();

    for entry in conflicting {
        if let Some(FileClassification::New(data)) = classify_as_new(entry.path, entry.hash) {
            results.new_files.push(data);
        }
    }
}

/// Compare scanned filesystem paths against the DB to find deleted files.
pub fn detect_deletions(results: &ScanResults, existing: &ExistingFiles) -> Vec<Uuid> {
    let mut known_paths: HashSet<&str> = HashSet::new();

    for p in &results.skipped {
        known_paths.insert(p);
    }
    for m in &results.moved {
        known_paths.insert(&m.path);
    }
    for n in &results.new_files {
        known_paths.insert(&n.path);
    }
    // Modified files still exist at their original path
    for m in &results.modified {
        known_paths.insert(&m.path);
    }

    existing
        .by_path
        .iter()
        .filter(|(path, _)| !known_paths.contains(path.as_str()))
        .map(|(_, (id, _))| *id)
        .collect()
}

/// Discover audio files and classify them in parallel against existing DB state.
pub fn classify_all(
    collection_path: &Path,
    existing: &ExistingFiles,
) -> ScanResults {
    let audio_files = get_audio_files(collection_path);

    let classifications: Vec<FileClassification> = audio_files
        .par_iter()
        .filter_map(|path| classify_file(path, existing))
        .collect();

    aggregate(classifications)
}
