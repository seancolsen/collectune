use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use uuid::Uuid;

use super::metadata::{extension_to_format, get_duration, get_track_metadata};
use super::types::{
    ExistingFiles, FileClassification, ModifiedEntry, MovedEntry, NewFileData, ScanResults,
};

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

/// Returns a normalized path string relative to `collection_root`, prefixed with `./`.
/// Falls back to the original path string if canonicalization fails.
fn normalize_path(path: &Path, canonical_root: &Path) -> String {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.strip_prefix(canonical_root).map_or_else(
        |_| path.to_string_lossy().to_string(),
        |rel| format!("./{}", rel.display()),
    )
}

fn hash_file(path: &Path) -> Option<[u8; 32]> {
    let data = fs::read(path).ok()?;
    Some(*blake3::hash(&data).as_bytes())
}

fn classify_file(
    path: &Path,
    existing: &ExistingFiles,
    canonical_root: &Path,
) -> Option<FileClassification> {
    let path_str = normalize_path(path, canonical_root);
    let meta = fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_micros() as i64;

    if let Some((_, _, existing_size, existing_mtime)) = existing.by_path.get(&path_str) {
        if size == *existing_size && mtime == *existing_mtime {
            return Some(FileClassification::Skipped { path: path_str });
        }

        // mtime or size changed -- hash to determine if content actually changed
        let hash = hash_file(path)?;
        let (id, existing_hash, _, _) = existing.by_path.get(&path_str).unwrap();

        if hash == *existing_hash {
            // Content identical; just mtime drifted. Record as modified so we
            // persist the new mtime (hash/size/duration will be unchanged).
            let duration = get_duration(path);
            return Some(FileClassification::Modified {
                id: *id,
                path: path_str,
                real_path: path.to_path_buf(),
                hash,
                size,
                duration,
                mtime,
            });
        }

        let duration = get_duration(path);
        return Some(FileClassification::Modified {
            id: *id,
            path: path_str,
            real_path: path.to_path_buf(),
            hash,
            size,
            duration,
            mtime,
        });
    }

    // Path not in DB -- hash to check for moves or treat as new
    let hash = hash_file(path)?;

    if let Some(entries) = existing.by_hash.get(&hash) {
        for (id, original_path) in entries {
            if !Path::new(original_path).exists() {
                return Some(FileClassification::Moved {
                    id: *id,
                    path: path_str,
                    mtime,
                });
            }
        }
    }

    classify_as_new(path, path_str, hash, mtime)
}

fn classify_as_new(
    real_path: &Path,
    path_str: String,
    hash: [u8; 32],
    mtime: i64,
) -> Option<FileClassification> {
    let ext = real_path.extension()?.to_str()?;
    let format = extension_to_format(ext)?;

    let (metadata, duration) = get_track_metadata(real_path)?;
    let size = fs::metadata(real_path).map_or(0, |m| m.len());

    Some(FileClassification::New(NewFileData {
        path: path_str,
        hash,
        size,
        duration,
        mtime,
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
            FileClassification::Moved { id, path, mtime } => {
                moved.push(MovedEntry { id, path, mtime });
            }
            FileClassification::Modified {
                id,
                path,
                real_path,
                hash,
                size,
                duration,
                mtime,
            } => modified.push(ModifiedEntry {
                id,
                path,
                real_path,
                hash,
                size,
                duration,
                mtime,
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
        if let Some(FileClassification::New(data)) =
            classify_as_new(&entry.real_path, entry.path, entry.hash, entry.mtime)
        {
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
    for m in &results.modified {
        known_paths.insert(&m.path);
    }

    existing
        .by_path
        .iter()
        .filter(|(path, _)| !known_paths.contains(path.as_str()))
        .map(|(_, (id, _, _, _))| *id)
        .collect()
}

/// Discover audio files and classify them in parallel against existing DB state.
pub fn classify_all(collection_path: &Path, existing: &ExistingFiles) -> ScanResults {
    let canonical_root =
        fs::canonicalize(collection_path).unwrap_or_else(|_| collection_path.to_path_buf());
    let audio_files = get_audio_files(collection_path);

    let classifications: Vec<FileClassification> = audio_files
        .par_iter()
        .filter_map(|path| classify_file(path, existing, &canonical_root))
        .collect();

    aggregate(classifications)
}
