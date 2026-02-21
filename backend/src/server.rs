use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use axum::routing::post;
use axum::Router;
use bytes::Bytes;
use arrow_ipc::writer::StreamWriter;
use duckdb::Connection;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;

struct AppState {
    db: Mutex<Connection>,
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

async fn query(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Response<Body> {
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(8);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

    tokio::task::spawn_blocking(move || {
        let conn = state.db.lock().unwrap();

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
        let writer = ChannelWriter { tx, buf: Vec::new() };
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

    match ready_rx.await {
        Ok(Ok(())) => {
            let stream = ReceiverStream::new(rx);
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/vnd.apache.arrow.stream")
                .body(Body::from_stream(stream))
                .unwrap()
        }
        Ok(Err(msg)) => Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(msg))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("query task panicked"))
            .unwrap(),
    }
}

pub async fn serve(conn: Connection) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState {
        db: Mutex::new(conn),
    });

    let app = Router::new()
        .route("/query", post(query))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = "0.0.0.0:3000";
    println!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
