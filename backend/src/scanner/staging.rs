use std::collections::HashMap;
use duckdb::params;
use duckdb::Connection;
use uuid::Uuid;

use super::types::{ExistingFiles, StagingData};

pub fn load_existing_artists(conn: &Connection) -> Result<HashMap<String, Uuid>, duckdb::Error> {
    let mut stmt = conn.prepare("SELECT id, name FROM artist")?;
    let rows = stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let name: String = row.get(1)?;
        Ok((name, id_str))
    })?;

    let mut map = HashMap::new();
    for row in rows {
        let (name, id_str) = row?;
        if let Ok(id) = Uuid::parse_str(&id_str) {
            map.insert(name, id);
        }
    }
    Ok(map)
}

pub fn load_existing_files(conn: &Connection) -> Result<ExistingFiles, duckdb::Error> {
    let mut stmt =
        conn.prepare("SELECT id, path, hash FROM file WHERE deletion IS NULL")?;
    let rows = stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let path: String = row.get(1)?;
        let hash_blob: Vec<u8> = row.get(2)?;
        Ok((id_str, path, hash_blob))
    })?;

    let mut by_path = HashMap::new();
    let mut by_hash: HashMap<[u8; 32], Vec<(Uuid, String)>> = HashMap::new();

    for row in rows {
        let (id_str, path, hash_blob) = row?;
        let Ok(id) = Uuid::parse_str(&id_str) else {
            continue;
        };
        let Ok(hash): Result<[u8; 32], _> = hash_blob.try_into() else {
            continue;
        };
        by_path.insert(path.clone(), (id, hash));
        by_hash.entry(hash).or_default().push((id, path));
    }

    Ok(ExistingFiles { by_path, by_hash })
}

pub fn create_staging_tables(conn: &Connection) -> Result<(), duckdb::Error> {
    conn.execute_batch(
        "
        CREATE TEMP TABLE staging_artist (id UUID, name TEXT);
        CREATE TEMP TABLE staging_album (id UUID, title TEXT, year USMALLINT);
        CREATE TEMP TABLE staging_file (
            id UUID, path TEXT, hash BLOB, size UINTEGER,
            format format, duration REAL
        );
        CREATE TEMP TABLE staging_track (
            id UUID, file UUID, title TEXT, album UUID,
            disc_number UTINYINT, track_number UTINYINT, genre TEXT
        );
        CREATE TEMP TABLE staging_credit (track UUID, artist UUID, ord REAL, role TEXT);
        CREATE TEMP TABLE staging_moved (id UUID, new_path TEXT);
        CREATE TEMP TABLE staging_modified (id UUID, hash BLOB, size UINTEGER, duration REAL);
        CREATE TEMP TABLE staging_deleted (file_id UUID, deletion_id UUID);
        ",
    )
}

pub fn insert_staging_data(conn: &Connection, data: &StagingData) -> Result<(), duckdb::Error> {
    {
        let mut app = conn.appender("staging_artist")?;
        for a in &data.artists {
            app.append_row(params![a.id.to_string(), a.name])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_album")?;
        for a in &data.albums {
            let year: Option<u16> = a.year;
            app.append_row(params![a.id.to_string(), a.title, year])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_file")?;
        for f in &data.files {
            app.append_row(params![
                f.id.to_string(),
                f.path,
                f.hash.as_slice(),
                f.size as u32,
                f.format,
                f.duration as f32,
            ])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_track")?;
        for t in &data.tracks {
            let album: Option<String> = t.album.map(|u| u.to_string());
            let disc: Option<u8> = t.disc_number;
            let track_num: Option<u8> = t.track_number;
            app.append_row(params![
                t.id.to_string(),
                t.file.to_string(),
                t.title,
                album,
                disc,
                track_num,
                t.genre,
            ])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_credit")?;
        for c in &data.credits {
            let role: Option<&str> = c.role.as_deref();
            app.append_row(params![
                c.track.to_string(),
                c.artist.to_string(),
                c.ord as f32,
                role,
            ])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_moved")?;
        for m in &data.moved {
            app.append_row(params![m.id.to_string(), m.new_path])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_modified")?;
        for m in &data.modified {
            app.append_row(params![
                m.id.to_string(),
                m.hash.as_slice(),
                m.size as u32,
                m.duration as f32,
            ])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_deleted")?;
        for d in &data.deleted {
            app.append_row(params![d.file_id.to_string(), d.deletion_id.to_string()])?;
        }
        app.flush()?;
    }

    Ok(())
}

const BATCH_SQL: &str = "
BEGIN TRANSACTION;

INSERT INTO artist (id, name) SELECT id, name FROM staging_artist;
INSERT INTO album (id, title, year) SELECT id, title, year FROM staging_album;

INSERT INTO file (id, path, hash, size, format, duration, added, deletion)
SELECT id, path, hash, size, format, duration, now(), NULL FROM staging_file;

INSERT INTO track (id, file, start_position, end_position, title, album,
                   disc_number, track_number, genre, rating)
SELECT id, file, NULL, NULL, title, album, disc_number, track_number, genre, NULL
FROM staging_track;

INSERT INTO credit (track, artist, ord, role)
SELECT track, artist, ord, role FROM staging_credit;

UPDATE file SET path = sm.new_path
FROM staging_moved sm WHERE file.id = sm.id;

UPDATE file SET hash = sm.hash, size = sm.size, duration = sm.duration
FROM staging_modified sm WHERE file.id = sm.id;

INSERT INTO deletion (id, timestamp)
SELECT DISTINCT deletion_id, now() FROM staging_deleted;

UPDATE file SET deletion = sd.deletion_id
FROM staging_deleted sd WHERE file.id = sd.file_id;

COMMIT;
";

pub fn execute_batch(conn: &Connection) -> Result<(), duckdb::Error> {
    conn.execute_batch(BATCH_SQL)
}
