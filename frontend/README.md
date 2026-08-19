# RadioCrate frontend

A [SolidJS](https://www.solidjs.com/) single-page app, built with
[Bun](https://bun.sh) + [Vite](https://vite.dev). In production it's compiled to
static assets (`frontend/dist`) and embedded into the `radiocrate` binary, which
serves them alongside the API under `/api`. For day-to-day UI work you don't need
any of that — see below.

## Decoupled development mode

You can iterate on the frontend against a **separately running, pre-compiled
backend** — no `cargo xtask`, no frontend rebuild, no re-embedding. Two
long-lived processes:

1. a compiled backend serving the API on `:3000`, and
2. the Vite dev server serving the UI on `:5173` with hot-module reload.

The Vite dev server proxies every `/api/*` request to the backend, so the
browser only ever talks to one origin (`localhost:5173`) and client code stays
origin-relative in dev and prod alike.

```
browser ──▶ localhost:5173 (Vite dev server)
              ├─ /            → SolidJS app, HMR
              └─ /api/*       → proxied to localhost:3000 (backend)
```

### One-time setup

```sh
# From the repo root: build & vendor the querydown-js WASM binding into
# frontend/vendor/, then install JS deps. Only needed once (or after the
# querydown rev or package.json changes).
cargo xtask build-release      # bootstraps frontend/vendor/querydown-js/ + runs bun install
```

If `frontend/vendor/querydown-js/` already exists, you can skip `xtask` entirely
and just install deps:

```sh
cd frontend
bun install
```

### 1. Start the backend (terminal 1)

Compile and run the standalone API server (the `backend` crate's
`radiocrate-server` binary). It serves the API but **not** the frontend, so you
never rebuild it while working on the UI — leave it running.

```sh
# From the repo root. Point it at any collection of audio files.
# --no-scan skips the startup collection scan for a faster boot.
cargo run -p backend -- serve /path/to/your/collection --no-scan
# → Listening on 0.0.0.0:3000
```

Use a release build if you want realistic query performance (debug DuckDB is
~10× slower):

```sh
cargo build --release -p backend
./target/release/radiocrate-server serve /path/to/your/collection --no-scan
```

### 2. Start the Vite dev server (terminal 2)

```sh
cd frontend
bun run dev
# → http://localhost:5173
```

Edit files under `src/` and the browser updates instantly via HMR. API calls hit
the backend from step 1 through the proxy.

## How the dev/prod API wiring stays consistent

The frontend always calls **origin-relative `/api/*`** URLs
([`src/api/rpc.ts`](src/api/rpc.ts), [`src/api/query.ts`](src/api/query.ts)) —
never a hardcoded host or port. Both runtimes make that path resolve to the same
handlers:

- **Production** — `radiocrate/src/main.rs` mounts the API with
  `.nest("/api", server::router(...))` and serves the embedded `dist` on every
  other path.
- **Dev** — the standalone `radiocrate-server` (`backend/src/main.rs` →
  `server::serve`) does the **same** `/api` nest, and Vite's proxy
  (`server.proxy` in [`vite.config.ts`](vite.config.ts)) forwards `/api` to
  `localhost:3000` with no path rewriting.

Because the dev backend mirrors the production path layout, there's nothing to
special-case in client code. The backend also applies a permissive CORS layer
(`CorsLayer::permissive()` in `backend/src/server.rs`), so pointing the frontend
directly at the backend from another origin works too if you ever bypass the
proxy.

**If you change the backend's port**, update the proxy target in
[`vite.config.ts`](vite.config.ts) (`server.proxy["/api"]`) to match, or pass
`--port 3000` to the backend to keep the default.

## Seeding UI state without a backend

For deterministic states (manual testing, Playwright), the app reads URL params
on startup ([`src/dev/seed.ts`](src/dev/seed.ts)) — a no-op in production when
absent:

```
http://localhost:5173/?sidebar=open&tabs=Lemonade,Deep%20Cuts
http://localhost:5173/?results=<url-encoded%20plain%20text>   # canned results, bypasses /api/query
http://localhost:5173/?tabs=Lemonade&palette=1                # command palette open (or ?palette=<text>)
http://localhost:5173/?tabs=Lemonade&shortcuts=1              # keyboard-shortcuts editor open
http://localhost:5173/?tabs=Lemonade&grid=lemonade&records=track,album&recordFixture=1
                                                              # right-click a row → "Edit track": the
                                                              # record editor's form, fed by a canned
                                                              # schema + records (add &recordDelay=800
                                                              # to watch it load)
```

## Other commands

Run from `frontend/`:

| Command                      | What it does                                    |
| ---------------------------- | ----------------------------------------------- |
| `bun run dev`                | Vite dev server with HMR (this doc)             |
| `bun run build`              | Production build → `frontend/dist`              |
| `bun run preview`            | Serve the built `dist` locally                  |
| `bun run typecheck`          | `tsgo --noEmit`                                 |
| `bun run lint`               | ESLint + `eslint-plugin-solid`                  |
| `bun run format` / `:check`  | Prettier write / check                          |
| `bun run test:unit`          | Vitest unit tests                               |
| `bun run test:visual`        | Playwright whole-app screenshots (light + dark) |
| `bun run test:visual:update` | Regenerate the visual baselines                 |

For the full production build (frontend + embedded binary), use
`cargo xtask build-release` from the repo root. See the top-level
[`DEVELOPMENT.md`](../DEVELOPMENT.md) for the containerized workflow and visual
snapshot details.
