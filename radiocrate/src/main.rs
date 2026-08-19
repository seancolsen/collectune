use axum::Router;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use backend::{db, scanner, server};
use clap::{Args, Parser, Subcommand};
use rust_embed::Embed;
use std::path::{Path, PathBuf};

#[derive(Embed)]
#[folder = "../frontend/dist/"]
struct Assets;

/// This binary's version, for `--version` and for `app.version`. `GIT_HASH` is
/// supplied by `build.rs`.
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")");

#[derive(Parser)]
#[command(name = "radiocrate")]
#[command(about = "RadioCrate — manage and play your audio collection")]
#[command(version = VERSION)]
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

/// The content type to serve `path` as.
///
/// `mime_guess` covers everything the build emits except the manifest, whose
/// `.webmanifest` extension it doesn't know; browsers want
/// `application/manifest+json` there.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("webmanifest") => "application/manifest+json",
        _ => mime_guess::from_path(path)
            .first_raw()
            .unwrap_or("application/octet-stream"),
    }
}

/// How long a browser may reuse `path` without asking.
///
/// Everything under `assets/` is emitted by Vite with a content hash in its file
/// name (`index-BsXt-K9s.js`), so those URLs are immutable by construction: a
/// changed file gets a new name and the old name is never reused. They can be
/// cached hard and forever.
///
/// Everything else keeps a stable URL across builds and must be revalidated.
/// `no-cache` still lets the browser keep its copy; it just has to ask, which
/// costs one conditional request and usually returns 304. `sw.js` is the one
/// that must not be gotten wrong — a browser reusing a cached service worker
/// never learns a new one exists, which disables the whole update mechanism.
/// See `specs/2026-08-pwa-updates/plan.md`.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

/// The frontend build embedded in this binary, as recorded by the frontend build
/// in `build-id.txt` (see `frontend/vite.config.ts`). Reported over
/// `app.version` so a running client can tell whether it came from this binary.
///
/// A missing file means the embedded `frontend/dist` was produced by a build
/// that predates the build-id plugin, which is a packaging fault rather than a
/// stale client. Falling back to the sentinel makes the check fail *open* — the
/// client skips the comparison instead of reporting itself permanently out of
/// date over a problem the user can't act on — and the warning puts the real
/// fault where whoever built the binary will see it.
fn embedded_build_id() -> String {
    match Assets::get("build-id.txt")
        .and_then(|file| String::from_utf8(file.data.to_vec()).ok())
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    {
        Some(id) => id,
        None => {
            eprintln!(
                "warning: no build-id.txt in the embedded frontend; \
                 client update detection is disabled. Rebuild with `cargo xtask build-release`."
            );
            backend::DEV_BUILD_ID.to_string()
        }
    }
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let asset_path = if path.is_empty() { "index.html" } else { path };

    // Unknown paths fall back to the shell so the app's own routing gets them —
    // but as `text/html`, not as whatever the requested extension implied. A
    // missing `.png` served as HTML-labelled-`image/png` is just a broken image
    // the browser can't explain.
    let (asset_path, file) = match Assets::get(asset_path) {
        Some(file) => (asset_path, file),
        None => match Assets::get("index.html") {
            Some(file) => ("index.html", file),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(asset_path)),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control(asset_path)),
    );
    if asset_path == "sw.js" {
        // Lets the worker control the whole origin regardless of where it's
        // served from. It's at the root here, so this is belt and braces — but
        // it costs nothing, and moving the file later would otherwise silently
        // narrow the scope.
        headers.insert(
            HeaderName::from_static("service-worker-allowed"),
            HeaderValue::from_static("/"),
        );
    }
    (headers, file.data).into_response()
}

/// Serves the API and the embedded frontend for a collection.
async fn serve(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (collection_path, conn) = open_collection(&args.collection)?;
    if !args.no_scan {
        scanner::scan(collection_path, &conn)?;
    }
    let state = server::app_state(
        conn,
        collection_path.to_path_buf(),
        embedded_build_id(),
        VERSION.to_string(),
    );

    let app = Router::new()
        .nest("/api", server::router(state))
        .fallback(static_handler);

    let addr = format!("0.0.0.0:{}", args.port);
    println!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Scan(args) => {
            let (collection_path, conn) = open_collection(&args.collection)?;
            scanner::scan(collection_path, &conn)?;
        }
        Command::Serve(args) => serve(args).await?,
    }
    Ok(())
}
