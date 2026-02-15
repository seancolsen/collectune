use clap::Parser;
use std::path::Path;

mod db;
mod scanner;

#[derive(Parser)]
#[command(name = "collectune")]
#[command(about = "A tool for managing audio file collections")]
struct Args {
    /// Path to the collection of audio files
    collection_path: String,
}

fn get_collection_path(path_str: &String) -> Result<&Path, String> {
    let path = Path::new(path_str);

    if !path.exists() {
        return Err(format!("The path '{}' does not exist.", path_str));
    }

    if !path.is_dir() {
        return Err(format!("The path '{}' is not a directory.", path_str));
    }

    Ok(path)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let collection_path = get_collection_path(&args.collection_path)?;
    let _db = db::get_db(collection_path)?;
    scanner::scan(collection_path);
    Ok(())
}
