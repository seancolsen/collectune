use std::sync::{Arc, Mutex};

use arrow_array::{Array, ArrayRef, LargeListArray, ListArray, RecordBatch, StringArray};
use arrow_buffer::Buffer;
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_ipc::reader::StreamDecoder;
use bytes::Bytes;
use eframe::egui;

use crate::QueryState;
use crate::now_playing::CurrentTrack;

#[cfg(target_arch = "wasm32")]
pub(crate) const BASE: &str = "/api";
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const BASE: &str = "http://localhost:3000";

pub(crate) fn run_query(query: String, state: &Arc<Mutex<QueryState>>, ctx: &egui::Context) {
    let handler = {
        let state = Arc::clone(state);
        let ctx = ctx.clone();
        move |batch: &RecordBatch| push_batch(batch, &state, &ctx)
    };
    let state_done = Arc::clone(state);
    let ctx_done = ctx.clone();
    let on_done = move |result: Result<(), String>| finish(result, &state_done, &ctx_done);
    stream_query(query, handler, on_done);
}

/// Fetches the database schema JSON once at startup and stores it in `schema`.
pub(crate) fn fetch_schema(schema: Arc<Mutex<Option<String>>>, ctx: egui::Context) {
    let store = move |json: String| {
        *schema.lock().unwrap() = Some(json);
        ctx.request_repaint();
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime");
            if let Ok(json) = rt.block_on(fetch_schema_native()) {
                store(json);
            }
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(json) = fetch_schema_wasm().await {
                store(json);
            }
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_schema_native() -> Result<String, String> {
    let url = format!("{BASE}/schema");
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_schema_wasm() -> Result<String, String> {
    let url = format!("{BASE}/schema");
    let resp = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(format!("{}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

pub(crate) fn fetch_track_metadata(
    track_id: &str,
    current_track: &Arc<Mutex<Option<CurrentTrack>>>,
    ctx: &egui::Context,
) {
    let sql = format!(
        "with a as (\
           select c.track, array_agg(ar.name order by c.ord) as artists \
           from credit c join artist ar on ar.id = c.artist \
           group by c.track\
         ) \
         select t.id::text as id, t.title, a.artists \
         from track t left join a on a.track = t.id \
         where t.id = TRY_CAST('{track_id}' as UUID);",
        track_id = track_id.replace('\'', "''"),
    );
    let track_id_for_handler = track_id.to_string();
    let current_track_for_handler = Arc::clone(current_track);
    let ctx_for_handler = ctx.clone();
    let handler = move |batch: &RecordBatch| {
        apply_metadata_batch(
            batch,
            &track_id_for_handler,
            &current_track_for_handler,
            &ctx_for_handler,
        );
        Ok::<(), String>(())
    };
    let on_done = |_result: Result<(), String>| {};
    stream_query(sql, handler, on_done);
}

fn extract_string_list(col: &ArrayRef) -> Vec<String> {
    let inner: Option<ArrayRef> = if let Some(list) = col.as_any().downcast_ref::<ListArray>() {
        (!list.is_null(0)).then(|| list.value(0))
    } else if let Some(list) = col.as_any().downcast_ref::<LargeListArray>() {
        (!list.is_null(0)).then(|| list.value(0))
    } else {
        None
    };
    let Some(inner) = inner else {
        return Vec::new();
    };
    let Some(strs) = inner.as_any().downcast_ref::<StringArray>() else {
        return Vec::new();
    };
    (0..strs.len())
        .filter(|i| !strs.is_null(*i))
        .map(|i| strs.value(i).to_string())
        .collect()
}

fn apply_metadata_batch(
    batch: &RecordBatch,
    track_id: &str,
    current_track: &Mutex<Option<CurrentTrack>>,
    ctx: &egui::Context,
) {
    if batch.num_rows() == 0 || batch.num_columns() < 3 {
        return;
    }
    let title = batch
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .filter(|a| !a.is_null(0))
        .map(|a| a.value(0).to_string());
    let artists = extract_string_list(batch.column(2));
    let mut guard = current_track.lock().unwrap();
    if let Some(ct) = guard.as_mut()
        && ct.id == track_id
    {
        ct.title = title;
        ct.artist_names = artists;
        drop(guard);
        ctx.request_repaint();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn stream_query<H, D>(query: String, handler: H, on_done: D)
where
    H: FnMut(&RecordBatch) -> Result<(), String> + Send + 'static,
    D: FnOnce(Result<(), String>) + Send + 'static,
{
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let result = rt.block_on(stream_query_native(&query, handler));
        on_done(result);
    });
}

#[cfg(target_arch = "wasm32")]
fn stream_query<H, D>(query: String, handler: H, on_done: D)
where
    H: FnMut(&RecordBatch) -> Result<(), String> + 'static,
    D: FnOnce(Result<(), String>) + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        let mut handler = handler;
        let result = stream_query_wasm(&query, &mut handler).await;
        on_done(result);
    });
}

fn finish(result: Result<(), String>, state: &Mutex<QueryState>, ctx: &egui::Context) {
    let mut s = state.lock().unwrap();
    if let Err(e) = result {
        s.error = Some(e);
    }
    s.running = false;
    drop(s);
    ctx.request_repaint();
}

fn push_batch(
    batch: &RecordBatch,
    state: &Mutex<QueryState>,
    ctx: &egui::Context,
) -> Result<(), String> {
    let fmt_opts = FormatOptions::default();
    let formatters: Vec<_> = batch
        .columns()
        .iter()
        .map(|col| ArrayFormatter::try_new(col.as_ref(), &fmt_opts))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut s = state.lock().unwrap();
    for row in 0..batch.num_rows() {
        let cells: Vec<String> = formatters
            .iter()
            .map(|fmt| fmt.value(row).to_string())
            .collect();
        s.rows.push(cells);
    }
    drop(s);
    ctx.request_repaint();
    Ok(())
}

fn feed_decoder<H>(decoder: &mut StreamDecoder, chunk: Bytes, handler: &mut H) -> Result<(), String>
where
    H: FnMut(&RecordBatch) -> Result<(), String>,
{
    let mut buf = Buffer::from(chunk);
    while !buf.is_empty() {
        match decoder.decode(&mut buf).map_err(|e| e.to_string())? {
            Some(batch) => handler(&batch)?,
            None => break,
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn stream_query_native<H>(query: &str, mut handler: H) -> Result<(), String>
where
    H: FnMut(&RecordBatch) -> Result<(), String>,
{
    use futures_util::StreamExt;

    let url = format!("{BASE}/query");
    let resp = reqwest::Client::new()
        .post(&url)
        .body(query.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let msg = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {msg}"));
    }

    let mut stream = resp.bytes_stream();
    let mut decoder = StreamDecoder::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        feed_decoder(&mut decoder, chunk, &mut handler)?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
async fn stream_query_wasm<H>(query: &str, handler: &mut H) -> Result<(), String>
where
    H: FnMut(&RecordBatch) -> Result<(), String>,
{
    use futures_util::StreamExt;
    use js_sys::Uint8Array;
    use wasm_bindgen::JsCast;
    use wasm_streams::ReadableStream;

    let url = format!("{BASE}/query");
    let resp = gloo_net::http::Request::post(&url)
        .body(query.to_string())
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        let status = resp.status();
        let msg = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {msg}"));
    }

    let body = resp
        .body()
        .ok_or_else(|| "response had no body".to_string())?;
    let mut stream = ReadableStream::from_raw(body.unchecked_into()).into_stream();
    let mut decoder = StreamDecoder::new();
    while let Some(chunk) = stream.next().await {
        let value = chunk.map_err(|e| format!("{e:?}"))?;
        let array: Uint8Array = value
            .dyn_into()
            .map_err(|_| "unexpected chunk type".to_string())?;
        let bytes = Bytes::from(array.to_vec());
        feed_decoder(&mut decoder, bytes, handler)?;
    }
    Ok(())
}
