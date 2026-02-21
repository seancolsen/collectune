use std::path::Path;

use duckdb::Connection;

use super::classify;
use super::prepare;
use super::staging;

pub fn scan(collection_path: &Path, conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let existing_artists = staging::load_existing_artists(conn)?;
    let existing_files = staging::load_existing_files(conn)?;

    let mut results = classify::classify_all(collection_path, &existing_files);

    println!(
        "Scan: {} skipped, {} moved, {} modified, {} new",
        results.skipped.len(),
        results.moved.len(),
        results.modified.len(),
        results.new_files.len(),
    );

    classify::resolve_conflicts(&mut results);

    let deleted_ids = classify::detect_deletions(&results, &existing_files);
    println!("Scan: {} deleted", deleted_ids.len());

    let staging_data = prepare::prepare_staging_data(&results, &existing_artists, deleted_ids);

    staging::create_staging_tables(conn)?;
    staging::insert_staging_data(conn, &staging_data)?;
    staging::execute_batch(conn)?;

    println!("Scan complete.");
    Ok(())
}
