use std::sync::{Arc, Mutex};

use arrow_array::RecordBatch;
use arrow_buffer::Buffer;
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_ipc::reader::StreamDecoder;
use bytes::Bytes;
use eframe::egui;

use crate::QueryState;

#[cfg(target_arch = "wasm32")]
const BASE: &str = "/api";
#[cfg(not(target_arch = "wasm32"))]
const BASE: &str = "http://localhost:3000";

pub(crate) fn run_query(query: String, state: Arc<Mutex<QueryState>>, ctx: egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime");
            let result = rt.block_on(stream_query_native(&query, &state, &ctx));
            finish(result, &state, &ctx);
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            let result = stream_query_wasm(&query, &state, &ctx).await;
            finish(result, &state, &ctx);
        });
    }
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

fn feed_decoder(
    decoder: &mut StreamDecoder,
    chunk: Bytes,
    state: &Mutex<QueryState>,
    ctx: &egui::Context,
) -> Result<(), String> {
    let mut buf = Buffer::from(chunk);
    while !buf.is_empty() {
        match decoder.decode(&mut buf).map_err(|e| e.to_string())? {
            Some(batch) => push_batch(&batch, state, ctx)?,
            None => break,
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn stream_query_native(
    query: &str,
    state: &Mutex<QueryState>,
    ctx: &egui::Context,
) -> Result<(), String> {
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
    ctx.request_repaint();

    let mut stream = resp.bytes_stream();
    let mut decoder = StreamDecoder::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        feed_decoder(&mut decoder, chunk, state, ctx)?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
async fn stream_query_wasm(
    query: &str,
    state: &Mutex<QueryState>,
    ctx: &egui::Context,
) -> Result<(), String> {
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
    ctx.request_repaint();

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
        feed_decoder(&mut decoder, bytes, state, ctx)?;
    }
    Ok(())
}
