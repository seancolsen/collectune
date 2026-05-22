use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use audiopus::coder::Encoder as OpusEncoder;
use audiopus::{Application, Bitrate, Channels as OpusChannels, SampleRate};
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use serde::Deserialize;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::server::AppState;

const OPUS_SAMPLE_RATE: u32 = 48_000;
const OPUS_FRAME_SAMPLES: usize = 960; // 20ms at 48kHz
const RESAMPLE_CHUNK_SIZE: usize = 1024;

#[derive(Deserialize)]
pub(crate) struct StreamParams {
    #[serde(default = "default_quality")]
    quality: String,
    #[serde(default)]
    start: f64,
}

fn default_quality() -> String {
    "original".to_string()
}

fn is_lossless(format: &str) -> bool {
    matches!(
        format,
        "flac" | "wav" | "aiff" | "alac" | "ape" | "wv" | "caf"
    )
}

fn format_content_type(format: &str) -> &'static str {
    match format {
        "aac" => "audio/aac",
        "adpcm" | "wav" => "audio/wav",
        "aiff" => "audio/aiff",
        "alac" | "mp4" => "audio/mp4",
        "ape" => "audio/x-ape",
        "caf" => "audio/x-caf",
        "flac" => "audio/flac",
        "mkv" => "audio/x-matroska",
        "mp1" | "mp2" | "mp3" => "audio/mpeg",
        "ogg" | "vorbis" | "opus" => "audio/ogg",
        "webm" => "audio/webm",
        "wma" => "audio/x-ms-wma",
        "wv" => "audio/x-wavpack",
        _ => "application/octet-stream",
    }
}

struct TrackFile {
    path: String,
    format: String,
}

fn lookup_track(state: &AppState, track_id: &str) -> Result<TrackFile, StatusCode> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT f.path, f.format \
             FROM track t JOIN file f ON t.file = f.id \
             WHERE t.id = TRY_CAST(? AS UUID)",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut rows = stmt
        .query_map([track_id], |row| {
            Ok(TrackFile {
                path: row.get(0)?,
                format: row.get(1)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    rows.next()
        .ok_or(StatusCode::NOT_FOUND)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn stream_track(
    State(state): State<Arc<AppState>>,
    AxumPath(track_id): AxumPath<String>,
    Query(params): Query<StreamParams>,
    request: Request,
) -> Response {
    let track = match lookup_track(&state, &track_id) {
        Ok(t) => t,
        Err(StatusCode::NOT_FOUND) => {
            return (StatusCode::NOT_FOUND, "track not found").into_response();
        }
        Err(status) => return (status, "database error").into_response(),
    };

    if !Path::new(&track.path).exists() {
        return (StatusCode::NOT_FOUND, "file not found on disk").into_response();
    }

    let should_transcode = params.quality == "opus128" && is_lossless(&track.format);

    if should_transcode {
        transcode_response(&track, params.start).await
    } else {
        passthrough_response(&track, request).await
    }
}

async fn passthrough_response(track: &TrackFile, request: Request) -> Response {
    let content_type = format_content_type(&track.format);

    let mut response = ServeFile::new(&track.path)
        .oneshot(request)
        .await
        .unwrap()
        .into_response();

    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type.parse().unwrap());

    response
}

async fn transcode_response(track: &TrackFile, start: f64) -> Response {
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(16);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
    let file_path = track.path.clone();

    tokio::task::spawn_blocking(move || {
        run_transcode_pipeline(&file_path, start, &tx, ready_tx);
    });

    match ready_rx.await {
        Ok(Ok(())) => {
            let stream = ReceiverStream::new(rx);
            let body = Body::from_stream(stream);
            ([(header::CONTENT_TYPE, "audio/ogg")], body).into_response()
        }
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "transcode task panicked").into_response(),
    }
}

/// Ensures setup errors are communicated to the handler via `ready_tx`
/// rather than silently dropping the channel.
fn run_transcode_pipeline(
    file_path: &str,
    start: f64,
    tx: &mpsc::Sender<io::Result<Bytes>>,
    ready_tx: oneshot::Sender<Result<(), String>>,
) {
    let mut ready_tx = Some(ready_tx);

    let result = transcode_inner(file_path, start, tx, &mut ready_tx);

    if let Err(ref e) = result {
        if let Some(ready) = ready_tx {
            let _ = ready.send(Err(e.to_string()));
        } else {
            eprintln!("transcode streaming error: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Ogg writer adapter: buffers Ogg page bytes and flushes them through the
// mpsc channel when flush() is called.
// ---------------------------------------------------------------------------

struct OggChannelWriter<'a> {
    tx: &'a mpsc::Sender<io::Result<Bytes>>,
    buf: Vec<u8>,
}

impl Write for OggChannelWriter<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let bytes = Bytes::from(std::mem::take(&mut self.buf));
            self.tx
                .blocking_send(Ok(bytes))
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "client disconnected"))?;
        }
        Ok(())
    }
}

impl Drop for OggChannelWriter<'_> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

// ---------------------------------------------------------------------------
// Ogg Opus header construction (RFC 7845)
// ---------------------------------------------------------------------------

fn build_opus_head(channels: u8, pre_skip: u16, original_sample_rate: u32) -> Vec<u8> {
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1); // version
    head.push(channels);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&original_sample_rate.to_le_bytes());
    head.extend_from_slice(&0_i16.to_le_bytes()); // output gain
    head.push(0); // channel mapping family (0 = mono/stereo)
    head
}

fn build_opus_tags() -> Vec<u8> {
    let vendor = b"collectune";
    let mut tags = Vec::with_capacity(8 + 4 + vendor.len() + 4);
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    tags.extend_from_slice(vendor);
    tags.extend_from_slice(&0_u32.to_le_bytes()); // zero user comments
    tags
}

// ---------------------------------------------------------------------------
// Transcode pipeline: Symphonia decode → resample → Opus encode → Ogg mux
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines, clippy::similar_names)]
fn transcode_inner(
    file_path: &str,
    start: f64,
    tx: &mpsc::Sender<io::Result<Bytes>>,
    ready_tx: &mut Option<oneshot::Sender<Result<(), String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // -- Open and probe source file --
    let file = std::fs::File::open(file_path)?;
    let mss = MediaSourceStream::new(
        Box::new(file),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );

    let mut hint = Hint::new();
    if let Some(ext) = Path::new(file_path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe failed: {e}"))?;

    let mut format = probed.format;

    let track = format.default_track().ok_or("no audio track found")?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let source_rate = codec_params.sample_rate.ok_or("unknown sample rate")?;
    let channels = codec_params
        .channels
        .ok_or("unknown channel layout")?
        .count();

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder creation failed: {e}"))?;

    if start > 0.0 {
        let time = symphonia::core::units::Time {
            seconds: start as u64,
            frac: start.fract(),
        };
        format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(track_id),
                },
            )
            .map_err(|e| format!("seek failed: {e}"))?;
    }

    // -- Create Opus encoder --
    let opus_channels = if channels == 1 {
        OpusChannels::Mono
    } else {
        OpusChannels::Stereo
    };
    let mut opus = OpusEncoder::new(SampleRate::Hz48000, opus_channels, Application::Audio)
        .map_err(|e| format!("opus encoder creation failed: {e}"))?;
    opus.set_bitrate(Bitrate::BitsPerSecond(128_000))
        .map_err(|e| format!("set bitrate failed: {e}"))?;

    let pre_skip = opus.lookahead().unwrap_or(312) as u16;

    // -- Create resampler if source rate differs from 48kHz --
    let needs_resample = source_rate != OPUS_SAMPLE_RATE;
    let mut resampler = if needs_resample {
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            oversampling_factor: 256,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::BlackmanHarris2,
        };
        Some(
            SincFixedIn::<f32>::new(
                f64::from(OPUS_SAMPLE_RATE) / f64::from(source_rate),
                2.0,
                params,
                RESAMPLE_CHUNK_SIZE,
                channels,
            )
            .map_err(|e| format!("resampler creation failed: {e}"))?,
        )
    } else {
        None
    };

    // -- Setup complete: signal the handler to start streaming --
    if let Some(ready) = ready_tx.take() {
        let _ = ready.send(Ok(()));
    }

    // -- Write Ogg Opus headers --
    let serial: u32 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();

    let ogg_writer = OggChannelWriter {
        tx,
        buf: Vec::new(),
    };
    {
        let mut ogg = PacketWriter::new(ogg_writer);

        let opus_head = build_opus_head(channels as u8, pre_skip, source_rate);
        ogg.write_packet(opus_head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let opus_tags = build_opus_tags();
        ogg.write_packet(opus_tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        ogg.inner_mut().flush()?;

        // -- Decode/resample/encode loop --
        let mut planar_buf: Vec<Vec<f32>> = vec![Vec::new(); channels];
        let mut interleaved_buf: Vec<f32> = Vec::new();
        let mut sample_buf: Option<SampleBuffer<f32>> = None;
        let mut encode_out = vec![0u8; 4000];
        let mut granule_pos: u64 = 0;

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(_) => break,
            };

            if packet.track_id() != track_id {
                continue;
            }

            let Ok(decoded) = decoder.decode(&packet) else {
                continue;
            };

            let spec = *decoded.spec();
            let frames = decoded.capacity();

            if sample_buf
                .as_ref()
                .is_none_or(|b| b.len() < frames * spec.channels.count())
            {
                sample_buf = Some(SampleBuffer::new(frames as u64, spec));
            }
            let buf = sample_buf.as_mut().unwrap();
            buf.copy_interleaved_ref(decoded);
            let samples = buf.samples();

            if needs_resample {
                // De-interleave into planar buffers for rubato
                for (i, &s) in samples.iter().enumerate() {
                    planar_buf[i % channels].push(s);
                }

                // Process full chunks through the resampler
                if let Some(ref mut rs) = resampler {
                    while planar_buf[0].len() >= RESAMPLE_CHUNK_SIZE {
                        let chunk: Vec<&[f32]> = planar_buf
                            .iter()
                            .map(|ch| &ch[..RESAMPLE_CHUNK_SIZE])
                            .collect();
                        let resampled = rs
                            .process(&chunk, None)
                            .map_err(|e| format!("resample failed: {e}"))?;
                        for ch in &mut planar_buf {
                            ch.drain(..RESAMPLE_CHUNK_SIZE);
                        }
                        interleave_into(&resampled, &mut interleaved_buf);
                    }
                }
            } else {
                interleaved_buf.extend_from_slice(samples);
            }

            // Encode complete Opus frames
            encode_frames(
                &mut interleaved_buf,
                channels,
                &opus,
                &mut encode_out,
                &mut granule_pos,
                &mut ogg,
                serial,
                false,
            )?;
        }

        // -- Flush remaining samples through the resampler --
        if let Some(ref mut rs) = resampler
            && !planar_buf[0].is_empty()
        {
            let chunk: Vec<&[f32]> = planar_buf.iter().map(|ch| &ch[..]).collect();
            let output = rs
                .process_partial(Some(&chunk), None)
                .map_err(|e| format!("resample flush failed: {e}"))?;
            interleave_into(&output, &mut interleaved_buf);
        }

        // Pad last frame with silence and encode
        let frame_samples = OPUS_FRAME_SAMPLES * channels;
        if !interleaved_buf.is_empty() {
            interleaved_buf.resize(frame_samples, 0.0);
            encode_frames(
                &mut interleaved_buf,
                channels,
                &opus,
                &mut encode_out,
                &mut granule_pos,
                &mut ogg,
                serial,
                true,
            )?;
        }

        ogg.inner_mut().flush()?;
    }
    // ogg and ogg_writer are dropped here, flushing any remaining data

    Ok(())
}

fn interleave_into(planar: &[Vec<f32>], out: &mut Vec<f32>) {
    if planar.is_empty() {
        return;
    }
    let frames = planar[0].len();
    out.reserve(frames * planar.len());
    for i in 0..frames {
        for ch in planar {
            out.push(ch[i]);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_frames(
    interleaved_buf: &mut Vec<f32>,
    channels: usize,
    opus: &OpusEncoder,
    encode_out: &mut [u8],
    granule_pos: &mut u64,
    ogg: &mut PacketWriter<'_, OggChannelWriter<'_>>,
    serial: u32,
    is_last: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frame_samples = OPUS_FRAME_SAMPLES * channels;

    while interleaved_buf.len() >= frame_samples {
        let frame = &interleaved_buf[..frame_samples];
        let encoded_len = opus
            .encode_float(frame, encode_out)
            .map_err(|e| format!("opus encode failed: {e}"))?;

        *granule_pos += OPUS_FRAME_SAMPLES as u64;

        let at_end = is_last && interleaved_buf.len() <= frame_samples;
        let end_info = if at_end {
            PacketWriteEndInfo::EndStream
        } else {
            PacketWriteEndInfo::EndPage
        };

        ogg.write_packet(
            encode_out[..encoded_len].to_vec(),
            serial,
            end_info,
            *granule_pos,
        )?;

        interleaved_buf.drain(..frame_samples);
    }

    ogg.inner_mut().flush()?;

    Ok(())
}
