# Collectune

A client-server app for managing and playing your personal collection of music files.

## Workspace layout

| Crate | Kind | Purpose |
|---|---|---|
| [`backend`](backend) | lib + bin (`collectune-server`) | Server logic (axum, DuckDB, scanner, audio stream). The bin is the dev API server. |
| [`frontend`](frontend) | lib + bin (`collectune-ui`) | egui app. The lib is shared between the native desktop bin and the WASM build. |
| [`collectune`](collectune) | bin (`collectune`) | **Production single binary** — depends on the `backend` lib and embeds the WASM frontend. |
| [`xtask`](xtask) | bin | Build orchestration (`cargo xtask build-release`). |

## Development

Build artifacts are split into two binaries so the UI can iterate without rebuilding the backend (and vice versa). The native UI talks to the API over HTTP on `localhost:3000`.

### Run the API server

```sh
cargo run -p backend -- /path/to/music
```

Options:

- `--port <PORT>` (default `3000`)
- `--no-scan` — skip the full collection scan on startup

### Run the native desktop UI

In a separate terminal:

```sh
cargo run -p frontend
```

Options:

- `--scale <FLOAT>` — UI scale factor (e.g. `--scale 1.5`)

The desktop UI sends queries to `http://localhost:3000` and streams Arrow IPC responses back.

## Production build

The production binary is a single executable that starts a web server, serves the API under `/api/*`, and serves the egui frontend (compiled to WASM) at `/`. All static assets (HTML, JS shim, WASM, etc.) are embedded into the binary.

### One-time setup

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

### Build

```sh
cargo xtask build-release
```

This runs two steps:

1. `trunk build --release` in [frontend/](frontend) — compiles the egui app to WASM and emits `frontend/dist/`.
2. `cargo build --release -p collectune` — builds the production binary, embedding `frontend/dist/` via `rust-embed`.

The resulting binary is at `target/release/collectune`.

### Run

```sh
./target/release/collectune /path/to/music
```

Options match the dev API server (`--port`, `--no-scan`). The web UI is served at `http://localhost:<port>/`; the API at `http://localhost:<port>/api/*`.

### Clean the WASM build

```sh
cargo xtask clean-web
```

## Code formatting

Uses `rustfmt` with project-specific settings in [rustfmt.toml](rustfmt.toml).

```sh
cargo fmt
```

To check formatting without modifying files:

```sh
cargo fmt --check
```

## Linting

Uses `clippy` with workspace-level lint rules defined in [Cargo.toml](Cargo.toml).

```sh
cargo clippy
```
