use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arrow_ipc::writer::StreamWriter;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, StatusCode, header};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use bytes::Bytes;
use duckdb::Connection;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;

pub struct AppState {
    db: Mutex<Connection>,
    pub collection_path: PathBuf,
    /// Identifies the frontend build this binary serves, reported over
    /// `app.version` so a client can tell whether it's the one that came with
    /// this binary. Supplied by the caller rather than read here: the embedded
    /// assets belong to the `radiocrate` crate, not to this one.
    pub build_id: String,
    /// This binary's version string, for display only.
    pub server_version: String,
}

impl AppState {
    /// Run a read-only DB operation under the connection lock.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        let conn = self.db.lock().unwrap();
        f(&conn)
    }

    /// Run a data-modifying DB operation under the lock, then `CHECKPOINT`.
    ///
    /// On success the WAL has been flushed to the main database file. Every
    /// mutation must flow through this method so the checkpoint can never be
    /// forgotten; this is why [`AppState::db`] is private.
    ///
    /// If the mutation succeeds but the `CHECKPOINT` fails, this returns
    /// `Err("checkpoint failed after write: ...")`. The mutation is already
    /// committed and durable in the WAL, so that error is not a signal to retry
    /// the write.
    pub fn write<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let conn = self.db.lock().unwrap();
        let value = f(&conn)?;
        conn.execute_batch("CHECKPOINT;")
            .map_err(|e| format!("checkpoint failed after write: {e}"))?;
        Ok(value)
    }

    /// Like [`AppState::write`], but hands the closure a `&mut Connection` so it can open a
    /// transaction (`DuckDB`'s `Connection::transaction` requires `&mut`). Used by the DML API, which
    /// runs a whole batch of operations inside one `BEGIN … COMMIT` and then checkpoints once.
    ///
    /// The same checkpoint invariant applies: on success the WAL is flushed to the main file. The
    /// error type is generic so callers can carry structured errors (e.g. [`crate::rpc::RpcErr`]); a
    /// checkpoint failure is surfaced through `E: From<String>`.
    pub fn write_mut<T, E: From<String>>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut conn = self.db.lock().unwrap();
        let value = f(&mut conn)?;
        conn.execute_batch("CHECKPOINT;")
            .map_err(|e| E::from(format!("checkpoint failed after write: {e}")))?;
        Ok(value)
    }
}

pub fn app_state(
    conn: Connection,
    collection_path: PathBuf,
    build_id: String,
    server_version: String,
) -> Arc<AppState> {
    let collection_path = std::fs::canonicalize(&collection_path).unwrap_or(collection_path);
    Arc::new(AppState {
        db: Mutex::new(conn),
        collection_path,
        build_id,
        server_version,
    })
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/query", post(query))
        .route("/rpc", post(crate::rpc::rpc))
        .route("/tracks/{id}/stream", get(crate::stream::stream_track))
        .route("/reauth", get(reauth))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Sends a browser back to the app once an authenticating reverse proxy has let
/// it through — the client's way out of a login it cannot otherwise reach.
///
/// A PWA behind a redirect-based identity proxy (Cloudflare Access here, but
/// oauth2-proxy and friends work the same way) has a bootstrapping problem: the
/// service worker answers navigations from its precache, so the proxy never gets
/// to redirect one to its sign-in page. All that reaches the network are
/// `fetch`es, and a `fetch` cannot render a sign-in page — it just fails. The
/// session cannot be renewed from inside the app, no matter how many times it is
/// reopened.
///
/// So the client navigates here instead. This path lives under `/api`, which the
/// service worker is configured never to answer (see
/// `navigateFallbackDenylist`), so the request reaches the proxy, which
/// intercepts it, authenticates the browser and sends it back — at which point
/// this handler bounces it to the app root. The response body is never seen; the
/// redirect is the whole payload.
///
/// With nothing in front of us it is simply a redirect to `/`, which is what
/// lets the client call it without knowing whether it is proxied.
async fn reauth() -> impl IntoResponse {
    // `no-store` because a cached bounce would skip the trip to the proxy that
    // is the entire point of this endpoint.
    (
        [(header::CACHE_CONTROL, "no-store")],
        // 303: this is a redirect to a different resource, not a relocation of
        // `/api/reauth` itself, and it must be followed with GET.
        Redirect::to("/"),
    )
}

/// Bridges synchronous Arrow IPC writes to an async byte stream.
///
/// Arrow's `StreamWriter` requires a synchronous `Write` target. This adapter
/// buffers incoming writes and flushes completed chunks through a tokio mpsc
/// channel, which the Axum handler consumes as a streaming HTTP response body.
struct ChannelWriter {
    tx: mpsc::Sender<io::Result<Bytes>>,
    buf: Vec<u8>,
}

impl ChannelWriter {
    fn send_buffered(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let bytes = Bytes::from(std::mem::take(&mut self.buf));
            self.tx
                .blocking_send(Ok(bytes))
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "client disconnected"))?;
        }
        Ok(())
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_buffered()
    }
}

impl Drop for ChannelWriter {
    fn drop(&mut self) {
        let _ = self.send_buffered();
    }
}

async fn query(State(state): State<Arc<AppState>>, body: String) -> Response<Body> {
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(8);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

    tokio::task::spawn_blocking(move || {
        state.read(|conn| {
            let mut stmt = match conn.prepare(&body) {
                Ok(stmt) => stmt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };

            let batches = match stmt.query_arrow([]) {
                Ok(b) => b,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };

            let schema = batches.get_schema();
            let _ = ready_tx.send(Ok(()));

            // Past this point, errors during streaming simply truncate the
            // response. The client will detect the missing IPC EOS marker.
            let writer = ChannelWriter {
                tx,
                buf: Vec::new(),
            };
            let Ok(mut ipc_writer) = StreamWriter::try_new(writer, &schema) else {
                return;
            };
            for batch in batches {
                if ipc_writer.write(&batch).is_err() {
                    return;
                }
            }
            let _ = ipc_writer.finish();
        });
    });

    match ready_rx.await {
        Ok(Ok(())) => {
            let stream = ReceiverStream::new(rx);
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/vnd.apache.arrow.stream")
                .body(Body::from_stream(stream))
                .unwrap()
        }
        Ok(Err(msg)) => {
            eprintln!("query: {msg}");
            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(msg))
                .unwrap()
        }
        Err(_) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("query task panicked"))
            .unwrap(),
    }
}

pub async fn serve(
    conn: Connection,
    collection_path: PathBuf,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    // Nest under `/api` so this standalone (frontend-less) dev server exposes the
    // same paths the production `radiocrate` binary does (which does the same nest
    // in its own `main.rs`). That lets the Vite dev server proxy `/api` straight
    // here with no path rewriting, and keeps client URLs origin-relative and
    // identical across dev and prod. See `frontend/README.md`.
    // `DEV_BUILD_ID` because this server embeds no frontend: the client it talks
    // to is whatever the Vite dev server is serving, which is by definition
    // current. Reporting the sentinel tells the client to skip the staleness
    // check rather than compare against an id that can't ever match.
    let app = Router::new().nest(
        "/api",
        router(app_state(
            conn,
            collection_path,
            api_schema::DEV_BUILD_ID.to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        )),
    );
    let addr = format!("0.0.0.0:{port}");
    println!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
