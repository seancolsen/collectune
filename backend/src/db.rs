use duckdb::Connection;
use std::path::Path;

static DB_FILE_NAME: &'static str = "collectune.db";

struct Migration {
    version: u32,
    sql: &'static str,
}

/// All known migrations, embedded at compile time.
///
/// To add a new migration, create a SQL file in this directory named with a
/// four-digit version prefix (e.g. `0002.sql`) and append a corresponding
/// entry here. Migrations must be listed in strictly ascending order.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("migrations/0001.sql"),
}];

fn init_db_version_metadata(conn: &Connection) -> Result<(), duckdb::Error> {
    let sql = "
        CREATE SCHEMA IF NOT EXISTS meta;
        CREATE TABLE IF NOT EXISTS meta.version (value UINTEGER NOT NULL);
        INSERT INTO meta.version SELECT 0 WHERE NOT EXISTS (SELECT 1 FROM meta.version);
    ";
    conn.execute_batch(sql)
}

fn get_current_version(conn: &Connection) -> Result<u32, duckdb::Error> {
    conn.query_row("SELECT value FROM meta.version", [], |row| row.get(0))
}

fn run_migration(conn: &mut Connection, migration: &Migration) -> Result<(), duckdb::Error> {
    let tx = conn.transaction()?;
    tx.execute_batch(migration.sql)?;
    tx.execute("UPDATE meta.version SET value = ?", [migration.version])?;
    tx.commit()?;
    println!("Migration {:04} applied.", migration.version);
    Ok(())
}

pub fn get_db(collection_path: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    let db_path = collection_path.join(DB_FILE_NAME);
    let mut conn = Connection::open(&db_path)?;
    init_db_version_metadata(&conn)?;
    let current_version = get_current_version(&conn)?;
    let pending_migrations = MIGRATIONS
        .iter()
        .filter(|m| m.version > current_version)
        .collect::<Vec<_>>();

    for migration in pending_migrations {
        run_migration(&mut conn, &migration)?;
    }

    Ok(conn)
}
