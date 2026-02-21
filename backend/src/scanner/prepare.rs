use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::types::{
    ScanResults, StagingAlbum, StagingArtist, StagingCredit, StagingData, StagingDeleted,
    StagingFile, StagingModified, StagingMoved, StagingTrack,
};

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
        parent.parent().map(std::path::Path::to_path_buf)
    } else {
        Some(parent.to_path_buf())
    }
}

fn collect_artists(
    results: &ScanResults,
    existing_artists: &HashMap<String, Uuid>,
) -> (HashMap<String, Uuid>, Vec<StagingArtist>) {
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
    (all_artists, new_artist_records)
}

fn collect_albums(results: &ScanResults) -> (HashMap<(String, PathBuf), Uuid>, Vec<StagingAlbum>) {
    let mut album_map: HashMap<(String, PathBuf), Uuid> = HashMap::new();
    let mut album_years: HashMap<Uuid, Option<u16>> = HashMap::new();

    for nf in &results.new_files {
        let album_dir = album_directory(Path::new(&nf.path)).unwrap_or_default();
        let key = (nf.metadata.album.clone(), album_dir);
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

    (album_map, staging_albums)
}

fn collect_changes(
    results: &ScanResults,
    deleted_ids: Vec<Uuid>,
) -> (Vec<StagingMoved>, Vec<StagingModified>, Vec<StagingDeleted>) {
    let staging_moved: Vec<StagingMoved> = results
        .moved
        .iter()
        .map(|m| StagingMoved {
            id: m.id,
            new_path: m.path.clone(),
            mtime: m.mtime,
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
            mtime: m.mtime,
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

    (staging_moved, staging_modified, staging_deleted)
}

pub fn prepare_staging_data(
    results: &ScanResults,
    existing_artists: &HashMap<String, Uuid>,
    deleted_ids: Vec<Uuid>,
) -> StagingData {
    let (all_artists, new_artist_records) = collect_artists(results, existing_artists);
    let (album_map, staging_albums) = collect_albums(results);

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
            mtime: nf.mtime,
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

    let (staging_moved, staging_modified, staging_deleted) = collect_changes(results, deleted_ids);

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
