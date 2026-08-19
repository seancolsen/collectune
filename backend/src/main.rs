use backend::{db, scanner, server};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "radiocrate-server")]
#[command(about = "A tool for managing audio file collections")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a collection into its database, then exit
    Scan(ScanArgs),

    /// Serve the web UI and API for a collection
    Serve(ServeArgs),
}

/// What every subcommand needs to know: which collection, and where its
/// database lives.
#[derive(Args)]
struct CollectionArgs {
    /// Path to the collection of audio files
    collection_path: String,

    /// Path to the database file (defaults to `radiocrate.db` in the collection root)
    #[arg(long)]
    db_path: Option<PathBuf>,
}

#[derive(Args)]
struct ScanArgs {
    #[command(flatten)]
    collection: CollectionArgs,
}

#[derive(Args)]
struct ServeArgs {
    #[command(flatten)]
    collection: CollectionArgs,

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
        return Err(format!("The path '{path_str}' does not exist."));
    }

    if !path.is_dir() {
        return Err(format!("The path '{path_str}' is not a directory."));
    }

    Ok(path)
}

/// Resolves the collection path and opens its database, migrating it if needed.
fn open_collection(
    args: &CollectionArgs,
) -> Result<(&Path, db::Connection), Box<dyn std::error::Error>> {
    let collection_path = get_collection_path(&args.collection_path)?;
    let db_path = args
        .db_path
        .clone()
        .unwrap_or_else(|| db::default_db_path(collection_path));
    let conn = db::get_db(&db_path)?;
    Ok((collection_path, conn))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Scan(args) => {
            let (collection_path, conn) = open_collection(&args.collection)?;
            scanner::scan(collection_path, &conn)?;
        }
        Command::Serve(args) => {
            let (collection_path, conn) = open_collection(&args.collection)?;
            if !args.no_scan {
                scanner::scan(collection_path, &conn)?;
            }
            server::serve(conn, collection_path.to_path_buf(), args.port).await?;
        }
    }
    Ok(())
}
