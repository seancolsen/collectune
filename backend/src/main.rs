use clap::Parser;
use std::path::Path;

mod db;
mod scanner;
mod server;

#[derive(Parser)]
#[command(name = "collectune")]
#[command(about = "A tool for managing audio file collections")]
struct Args {
    /// Path to the collection of audio files
    collection_path: String,

    /// Start without running a full collection scan
    #[arg(long)]
    no_scan: bool,

    /// Port to listen on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let collection_path = get_collection_path(&args.collection_path)?;
    let conn = db::get_db(collection_path)?;
    if !args.no_scan {
        scanner::scan(collection_path, &conn)?;
    }
    server::serve(conn, args.port).await?;
    Ok(())
}
