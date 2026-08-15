use duckdb::Connection;
use duckdb::params;
use std::collections::HashMap;
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
        conn.prepare("SELECT id, path, hash, size, mtime FROM file WHERE deletion IS NULL")?;
    let rows = stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let path: String = row.get(1)?;
        let hash_blob: Vec<u8> = row.get(2)?;
        let size: u32 = row.get(3)?;
        let mtime: i64 = row.get(4)?;
        Ok((id_str, path, hash_blob, u64::from(size), mtime))
    })?;

    let mut by_path = HashMap::new();
    let mut by_hash: HashMap<[u8; 32], Vec<(Uuid, String)>> = HashMap::new();

    for row in rows {
        let (id_str, path, hash_blob, size, mtime) = row?;
        let Ok(id) = Uuid::parse_str(&id_str) else {
            continue;
        };
        let Ok(hash): Result<[u8; 32], _> = hash_blob.try_into() else {
            continue;
        };
        by_path.insert(path.clone(), (id, hash, size, mtime));
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
            format format, duration REAL, mtime BIGINT
        );
        CREATE TEMP TABLE staging_track (
            id UUID, file UUID, title TEXT, album UUID,
            disc_number UTINYINT, track_number UTINYINT
        );
        CREATE TEMP TABLE staging_track_tag (track UUID, tag TEXT);
        CREATE TEMP TABLE staging_credit (track UUID, artist UUID, \"order\" REAL, role TEXT);
        CREATE TEMP TABLE staging_moved (id UUID, new_path TEXT, mtime BIGINT);
        CREATE TEMP TABLE staging_modified (id UUID, hash BLOB, size UINTEGER, duration REAL, mtime BIGINT);
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
                f.mtime,
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
            ])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_track_tag")?;
        for tt in &data.track_tags {
            app.append_row(params![tt.track.to_string(), tt.tag])?;
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
                c.order as f32,
                role,
            ])?;
        }
        app.flush()?;
    }

    {
        let mut app = conn.appender("staging_moved")?;
        for m in &data.moved {
            app.append_row(params![m.id.to_string(), m.new_path, m.mtime])?;
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
                m.mtime,
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

// The staging tables carry role and tag *names*, not ids: the scanner reads names out of the files
// and only this merge knows which of them the collection already has a record for. So each of the
// two shared-value tables gets any name it is missing, and the rows that reference it are then
// joined to it by name. `duration` arrives as a count of seconds and is widened to the INTERVAL the
// column now holds.
const BATCH_SQL: &str = "
BEGIN TRANSACTION;

INSERT INTO artist (id, name) SELECT id, name FROM staging_artist;
INSERT INTO album (id, title, year) SELECT id, title, year FROM staging_album;

INSERT INTO tag (id, name)
SELECT uuid(), tag FROM staging_track_tag
WHERE tag NOT IN (SELECT name FROM tag)
GROUP BY tag;

INSERT INTO role (id, name)
SELECT uuid(), role FROM staging_credit
WHERE role IS NOT NULL AND role NOT IN (SELECT name FROM role)
GROUP BY role;

INSERT INTO file (id, path, hash, size, format, duration, mtime, added, deletion)
SELECT id, path, hash, size, format, to_seconds(duration), mtime, now(), NULL FROM staging_file;

INSERT INTO track (id, file, start_position, end_position, title, album,
                   disc_number, track_number, rating)
SELECT id, file, NULL, NULL, title, album, disc_number, track_number, NULL
FROM staging_track;

INSERT INTO track_tag (track, tag)
SELECT DISTINCT stt.track, t.id
FROM staging_track_tag stt JOIN tag t ON t.name = stt.tag;

INSERT INTO credit (track, artist, \"order\", role)
SELECT sc.track, sc.artist, sc.\"order\", r.id
FROM staging_credit sc LEFT JOIN role r ON r.name = sc.role;

UPDATE file SET path = sm.new_path, mtime = sm.mtime
FROM staging_moved sm WHERE file.id = sm.id;

UPDATE file SET hash = sm.hash, size = sm.size, duration = to_seconds(sm.duration), mtime = sm.mtime
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

/// Drops the `staging_*` temp tables created by [`create_staging_tables`]. Called once the scan has
/// merged them into the real tables, so they don't linger on the connection for the rest of the
/// process's life (the connection is shared and long-lived — see `server::AppState`).
pub fn drop_staging_tables(conn: &Connection) -> Result<(), duckdb::Error> {
    conn.execute_batch(
        "
        DROP TABLE staging_artist;
        DROP TABLE staging_album;
        DROP TABLE staging_file;
        DROP TABLE staging_track;
        DROP TABLE staging_track_tag;
        DROP TABLE staging_credit;
        DROP TABLE staging_moved;
        DROP TABLE staging_modified;
        DROP TABLE staging_deleted;
        ",
    )
}
