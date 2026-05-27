use axum::Router;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use backend::{db, scanner, server};
use clap::Parser;
use rust_embed::Embed;
use std::path::Path;

#[derive(Embed)]
#[folder = "../frontend/dist/"]
struct Assets;

#[derive(Parser)]
#[command(name = "collectune")]
#[command(about = "Collectune — manage and play your audio collection")]
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

fn get_collection_path(path_str: &str) -> Result<&Path, String> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Err(format!("The path '{path_str}' does not exist."));
    }
    if !path.is_dir() {
        return Err(format!("The path '{path_str}' is not a directory."));
    }
    Ok(path)
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let asset_path = if path.is_empty() { "index.html" } else { path };
    let file = Assets::get(asset_path).or_else(|| Assets::get("index.html"));
    match file {
        Some(file) => {
            let mime = mime_guess::from_path(asset_path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let collection_path = get_collection_path(&args.collection_path)?;
    let conn = db::get_db(collection_path)?;
    if !args.no_scan {
        scanner::scan(collection_path, &conn)?;
    }
    let state = server::app_state(conn, collection_path.to_path_buf());

    let app = Router::new()
        .nest("/api", server::router(state))
        .fallback(static_handler);

    let addr = format!("0.0.0.0:{}", args.port);
    println!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
