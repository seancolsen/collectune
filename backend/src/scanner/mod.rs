mod metadata;
mod staging;
pub mod types;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use duckdb::Connection;
use rayon::prelude::*;
use uuid::Uuid;

use metadata::{extension_to_format, get_duration, get_track_metadata};
use types::*;

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
fn resolve_conflicts(results: &mut ScanResults) {
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
fn detect_deletions(results: &ScanResults, existing: &ExistingFiles) -> Vec<Uuid> {
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

static DISC_FOLDER_PATTERN: &[&str] = &["disc", "cd", "disk"];

fn is_disc_folder(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    DISC_FOLDER_PATTERN.iter().any(|prefix| {
        if let Some(rest) = lower.strip_prefix(prefix) {
            rest.trim().chars().all(|c| c.is_ascii_digit()) && !rest.trim().is_empty()
        } else {
            false
        }
    })
}

/// Determine the "album directory" for a file, looking through disc folders.
fn album_directory(file_path: &Path) -> Option<PathBuf> {
    let parent = file_path.parent()?;
    let dir_name = parent.file_name()?.to_str()?;

    if is_disc_folder(dir_name) {
        parent.parent().map(|p| p.to_path_buf())
    } else {
        Some(parent.to_path_buf())
    }
}

fn prepare_staging_data(
    results: &mut ScanResults,
    existing_artists: &HashMap<String, Uuid>,
    deleted_ids: Vec<Uuid>,
) -> StagingData {
    // --- Artists ---
    let mut all_artists: HashMap<String, Uuid> = existing_artists.clone();
    let mut new_artist_records: Vec<StagingArtist> = Vec::new();

    for nf in &results.new_files {
        for ta in &nf.metadata.artists {
            if !all_artists.contains_key(&ta.artist) {
                let id = Uuid::new_v4();
                all_artists.insert(ta.artist.clone(), id);
                new_artist_records.push(StagingArtist {
                    id,
                    name: ta.artist.clone(),
                });
            }
        }
    }

    // --- Albums (directory-based grouping) ---
    // Key: (album_title, album_directory) -> album UUID
    let mut album_map: HashMap<(String, PathBuf), Uuid> = HashMap::new();
    let mut album_years: HashMap<Uuid, Option<u16>> = HashMap::new();

    for nf in &results.new_files {
        let album_title = &nf.metadata.album;
        let album_dir = album_directory(Path::new(&nf.path)).unwrap_or_default();
        let key = (album_title.clone(), album_dir);

        let album_id = *album_map.entry(key).or_insert_with(Uuid::new_v4);
        album_years.entry(album_id).or_insert(nf.metadata.year);
    }

    let staging_albums: Vec<StagingAlbum> = album_map
        .iter()
        .map(|((title, _), &id)| StagingAlbum {
            id,
            title: title.clone(),
            year: album_years.get(&id).copied().flatten(),
        })
        .collect();

    // Build reverse lookup from (album_title, album_dir) -> uuid for file processing
    // (already in album_map)

    // --- Files, Tracks, Credits ---
    let mut staging_files: Vec<StagingFile> = Vec::new();
    let mut staging_tracks: Vec<StagingTrack> = Vec::new();
    let mut staging_credits: Vec<StagingCredit> = Vec::new();

    for nf in &results.new_files {
        let file_id = Uuid::new_v4();
        let track_id = Uuid::new_v4();

        staging_files.push(StagingFile {
            id: file_id,
            path: nf.path.clone(),
            hash: nf.hash,
            size: nf.size,
            format: nf.format.clone(),
            duration: nf.duration,
        });

        let album_dir = album_directory(Path::new(&nf.path)).unwrap_or_default();
        let album_key = (nf.metadata.album.clone(), album_dir);
        let album_id = album_map.get(&album_key).copied();

        staging_tracks.push(StagingTrack {
            id: track_id,
            file: file_id,
            title: nf.metadata.title.clone(),
            album: album_id,
            disc_number: nf.metadata.disc_number,
            track_number: nf.metadata.track_number,
            genre: nf.metadata.genre.clone(),
        });

        for (i, ta) in nf.metadata.artists.iter().enumerate() {
            if let Some(&artist_id) = all_artists.get(&ta.artist) {
                staging_credits.push(StagingCredit {
                    track: track_id,
                    artist: artist_id,
                    ord: i as f64,
                    role: ta.role.clone(),
                });
            }
        }
    }

    // --- Moved, Modified, Deleted ---
    let staging_moved: Vec<StagingMoved> = results
        .moved
        .iter()
        .map(|m| StagingMoved {
            id: m.id,
            new_path: m.path.clone(),
        })
        .collect();

    let staging_modified: Vec<StagingModified> = results
        .modified
        .iter()
        .map(|m| StagingModified {
            id: m.id,
            hash: m.hash,
            size: m.size,
            duration: m.duration,
        })
        .collect();

    let deletion_id = Uuid::new_v4();
    let staging_deleted: Vec<StagingDeleted> = deleted_ids
        .into_iter()
        .map(|file_id| StagingDeleted {
            file_id,
            deletion_id,
        })
        .collect();

    StagingData {
        artists: new_artist_records,
        albums: staging_albums,
        files: staging_files,
        tracks: staging_tracks,
        credits: staging_credits,
        moved: staging_moved,
        modified: staging_modified,
        deleted: staging_deleted,
    }
}

pub fn scan(
    collection_path: &Path,
    conn: &Connection,
) -> Result<(), Box<dyn std::error::Error>> {
    // Step 1: Load existing data from DB
    let existing_artists = staging::load_existing_artists(conn)?;
    let existing_files = staging::load_existing_files(conn)?;

    // Step 2: Discover audio files
    let audio_files = get_audio_files(collection_path);

    // Step 3: Parallel scanning with classification
    let classifications: Vec<FileClassification> = audio_files
        .par_iter()
        .filter_map(|path| classify_file(path, &existing_files))
        .collect();

    // Step 4: Aggregate results
    let mut results = aggregate(classifications);

    println!(
        "Scan: {} skipped, {} moved, {} modified, {} new",
        results.skipped.len(),
        results.moved.len(),
        results.modified.len(),
        results.new_files.len(),
    );

    // Step 5: Resolve moved-vs-modified conflicts
    resolve_conflicts(&mut results);

    // Step 6: Detect deletions
    let deleted_ids = detect_deletions(&results, &existing_files);
    println!("Scan: {} deleted", deleted_ids.len());

    // Step 7: Prepare staging data
    let staging_data = prepare_staging_data(&mut results, &existing_artists, deleted_ids);

    // Step 8: Create staging tables and insert data
    staging::create_staging_tables(conn)?;
    staging::insert_staging_data(conn, &staging_data)?;

    // Step 9: Execute batch transaction
    staging::execute_batch(conn)?;

    println!("Scan complete.");
    Ok(())
}
