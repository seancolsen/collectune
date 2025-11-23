use clap::Parser;
use std::path::Path;

#[derive(Parser)]
#[command(name = "collectune")]
#[command(about = "A tool for managing audio file collections")]
struct Args {
    /// Path to the collection of audio files
    collection_path: String,
}

fn main() {
    let args = Args::parse();

    let path = Path::new(&args.collection_path);
    
    if !path.exists() {
        eprintln!("Error: The path '{}' does not exist.", args.collection_path);
        std::process::exit(1);
    }
    
    if !path.is_dir() {
        eprintln!("Error: The path '{}' is not a directory.", args.collection_path);
        std::process::exit(1);
    }

    println!("{}", args.collection_path);
}
