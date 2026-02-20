use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug)]
pub struct TrackMetadata {
    pub title: String,
    pub track_number: Option<u8>,
    pub disc_number: Option<u8>,
    pub genre: String,
    pub album: String,
    pub year: Option<u16>,
    pub artists: Vec<TrackArtistMetadata>,
}

#[derive(Debug)]
pub struct TrackArtistMetadata {
    pub artist: String,
    pub role: Option<String>,
}

pub struct ExistingFiles {
    pub by_path: HashMap<String, (Uuid, [u8; 32], u64, i64)>, // id, hash, size, mtime_us
    pub by_hash: HashMap<[u8; 32], Vec<(Uuid, String)>>,
}

pub enum FileClassification {
    Skipped {
        path: String,
    },
    Moved {
        id: Uuid,
        path: String,
        mtime: i64,
    },
    Modified {
        id: Uuid,
        path: String,
        hash: [u8; 32],
        size: u64,
        duration: f64,
        mtime: i64,
    },
    New(NewFileData),
}

pub struct NewFileData {
    pub path: String,
    pub hash: [u8; 32],
    pub size: u64,
    pub duration: f64,
    pub mtime: i64,
    pub format: String,
    pub metadata: TrackMetadata,
}

pub struct MovedEntry {
    pub id: Uuid,
    pub path: String,
    pub mtime: i64,
}

pub struct ModifiedEntry {
    pub id: Uuid,
    pub path: String,
    pub hash: [u8; 32],
    pub size: u64,
    pub duration: f64,
    pub mtime: i64,
}

pub struct ScanResults {
    pub skipped: Vec<String>,
    pub moved: Vec<MovedEntry>,
    pub modified: Vec<ModifiedEntry>,
    pub new_files: Vec<NewFileData>,
}

pub struct StagingArtist {
    pub id: Uuid,
    pub name: String,
}

pub struct StagingAlbum {
    pub id: Uuid,
    pub title: String,
    pub year: Option<u16>,
}

pub struct StagingFile {
    pub id: Uuid,
    pub path: String,
    pub hash: [u8; 32],
    pub size: u64,
    pub format: String,
    pub duration: f64,
    pub mtime: i64,
}

pub struct StagingTrack {
    pub id: Uuid,
    pub file: Uuid,
    pub title: String,
    pub album: Option<Uuid>,
    pub disc_number: Option<u8>,
    pub track_number: Option<u8>,
    pub genre: String,
}

pub struct StagingCredit {
    pub track: Uuid,
    pub artist: Uuid,
    pub ord: f64,
    pub role: Option<String>,
}

pub struct StagingMoved {
    pub id: Uuid,
    pub new_path: String,
    pub mtime: i64,
}

pub struct StagingModified {
    pub id: Uuid,
    pub hash: [u8; 32],
    pub size: u64,
    pub duration: f64,
    pub mtime: i64,
}

pub struct StagingDeleted {
    pub file_id: Uuid,
    pub deletion_id: Uuid,
}

pub struct StagingData {
    pub artists: Vec<StagingArtist>,
    pub albums: Vec<StagingAlbum>,
    pub files: Vec<StagingFile>,
    pub tracks: Vec<StagingTrack>,
    pub credits: Vec<StagingCredit>,
    pub moved: Vec<StagingMoved>,
    pub modified: Vec<StagingModified>,
    pub deleted: Vec<StagingDeleted>,
}
