use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arrow_ipc::writer::StreamWriter;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use duckdb::Connection;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;

pub struct AppState {
    pub db: Mutex<Connection>,
    pub collection_path: PathBuf,
}

pub fn app_state(conn: Connection, collection_path: PathBuf) -> Arc<AppState> {
    let collection_path = std::fs::canonicalize(&collection_path).unwrap_or(collection_path);
    Arc::new(AppState {
        db: Mutex::new(conn),
        collection_path,
    })
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/query", post(query))
        .route("/schema", get(schema))
        .route("/tracks/{id}/stream", get(crate::stream::stream_track))
        .layer(CorsLayer::permissive())
        .with_state(state)
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

/// JSON schema describing the database, shaped for the Querydown compiler.
#[derive(serde::Serialize)]
struct SchemaJson {
    tables: Vec<TableJson>,
    links: Vec<LinkJson>,
}

#[derive(serde::Serialize)]
struct TableJson {
    name: String,
    columns: Vec<ColumnJson>,
}

#[derive(serde::Serialize)]
struct ColumnJson {
    name: String,
}

#[derive(serde::Serialize)]
struct LinkJson {
    from: RefJson,
    to: RefJson,
    unique: bool,
}

#[derive(serde::Serialize)]
struct RefJson {
    table: String,
    column: String,
}

/// Introspects the `DuckDB` database and returns its structure as Querydown schema JSON.
///
/// `DuckDB` has no foreign keys, so links are synthesized by convention: any `UUID`
/// column whose name matches another table's name is treated as a link to that
/// table's `id` column.
async fn schema(State(state): State<Arc<AppState>>) -> Response<Body> {
    let built = tokio::task::spawn_blocking(move || {
        let conn = state.db.lock().unwrap();
        build_schema(&conn)
    })
    .await;

    match built {
        Ok(Ok(schema)) => Json(schema).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "schema task panicked".to_string(),
        )
            .into_response(),
    }
}

fn build_schema(conn: &Connection) -> Result<SchemaJson, String> {
    use std::collections::{HashMap, HashSet};

    let mut stmt = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
        )
        .map_err(|e| e.to_string())?;
    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e: duckdb::Error| e.to_string())?;

    let table_set: HashSet<&str> = table_names.iter().map(String::as_str).collect();
    let index: HashMap<&str, usize> = table_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut tables: Vec<TableJson> = table_names
        .iter()
        .map(|name| TableJson {
            name: name.clone(),
            columns: Vec::new(),
        })
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT table_name, column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'main' \
             ORDER BY table_name, ordinal_position",
        )
        .map_err(|e| e.to_string())?;
    let columns: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e: duckdb::Error| e.to_string())?;

    let mut links = Vec::new();
    for (table, column, data_type) in &columns {
        // Skip columns belonging to views or other non-base-table relations.
        let Some(&table_index) = index.get(table.as_str()) else {
            continue;
        };
        tables[table_index].columns.push(ColumnJson {
            name: column.clone(),
        });
        if data_type.eq_ignore_ascii_case("UUID") && table_set.contains(column.as_str()) {
            links.push(LinkJson {
                from: RefJson {
                    table: table.clone(),
                    column: column.clone(),
                },
                to: RefJson {
                    table: column.clone(),
                    column: "id".to_string(),
                },
                unique: false,
            });
        }
    }

    Ok(SchemaJson { tables, links })
}

pub async fn serve(
    conn: Connection,
    collection_path: PathBuf,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(app_state(conn, collection_path));
    let addr = format!("0.0.0.0:{port}");
    println!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
