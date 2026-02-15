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

fn get_validated_collection_path(path_str: &String) -> &Path {
    let path = Path::new(path_str);

    if !path.exists() {
        eprintln!("Error: The path '{}' does not exist.", path_str);
        std::process::exit(1);
    }

    if !path.is_dir() {
        eprintln!("Error: The path '{}' is not a directory.", path_str);
        std::process::exit(1);
    }

    path
}

fn main() {
    let args = Args::parse();
    let collection_path = get_validated_collection_path(&args.collection_path);
    let _db = match db::get_db(collection_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error initializing database: {}", e);
            std::process::exit(1);
        }
    };
    scanner::scan(collection_path);
}
