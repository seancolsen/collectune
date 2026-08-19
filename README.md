# RadioCrate

A client-server app for managing and playing your personal collection of music files.

## Workspace layout

| Crate | Kind | Purpose |
|---|---|---|
| [`backend`](backend) | lib + bin (`radiocrate-server`) | Server logic (axum, DuckDB, scanner, audio stream). The bin is the dev API server. |
| [`frontend`](frontend) | SolidJS SPA (not a cargo crate) | **Production frontend** — a web-native DOM app built with SolidJS + Vite + Bun. Its `dist/` output is embedded by `radiocrate`. |
| [`radiocrate`](radiocrate) | bin (`radiocrate`) | **Production single binary** — depends on the `backend` lib and embeds the Solid frontend's `frontend/dist/`. |
| [`xtask`](xtask) | bin | Build orchestration (`cargo xtask build-release`). |

## Development

Build artifacts are split into two binaries so the UI can iterate without rebuilding the backend (and vice versa). The native UI talks to the API over HTTP on `localhost:3000`.

### Run the API server

```sh
cargo run -p backend -- serve /path/to/music
```

Options:

- `--port <PORT>` (default `3000`)
- `--no-scan` — skip the full collection scan on startup
- `--db-path <PATH>` (default `radiocrate.db` in the collection root)

To scan a collection into its database without serving anything, use `scan` in
place of `serve`. It takes the collection path and `--db-path`, and exits when
the scan finishes.

### Run the new SolidJS frontend (dev)

In a separate terminal:

```sh
cd frontend
bun install   # first time only
bun run dev
```

Vite serves the app with hot-module reload; it talks to the API server on
`http://localhost:3000`. Its checks are `bun run typecheck | lint | format:check | build`
and the Playwright visual snapshots (`bun run test:visual`).

## Production build

The production binary is a single executable that starts a web server, serves the API under `/api/*`, and serves the SolidJS frontend at `/`. All static assets (HTML, JS, CSS, icons, service worker, etc.) are embedded into the binary.

### One-time setup

Install [Bun](https://bun.sh) (the frontend's package manager and runtime):

```sh
curl -fsSL https://bun.sh/install | bash
```

### Build

```sh
cargo xtask build-release
```

This runs:

1. `bun install` then `bun run build` in [frontend/](frontend) — builds the Solid app with Vite and emits `frontend/dist/` (including a Workbox-generated `sw.js`; precache revisioning is handled by `vite-plugin-pwa`, so there is no separate stamping step).
2. `cargo build --release -p radiocrate` — builds the production binary, embedding `frontend/dist/` via `rust-embed`.

The resulting binary is at `target/release/radiocrate`.

### Run

```sh
./target/release/radiocrate serve /path/to/music
```

Subcommands and options match the dev API server (`serve` / `scan`, `--port`,
`--no-scan`, `--db-path`). The web UI is served at `http://localhost:<port>/`; the API at `http://localhost:<port>/api/*`.

### Clean the frontend build

```sh
cargo xtask clean-web
```

## Installing as an app

RadioCrate is a Progressive Web App: from a browser you can install it to a home
screen, dock or app launcher, after which it runs in its own window with no
browser chrome, its own icon and launch screen, and a service worker that caches
the app itself so it starts without waiting on the network. Playback keeps
driving the OS media controls — lock screen, notification shade, headset
buttons — as it already did in a tab.

### It has to be served over HTTPS

Browsers only allow service workers and installation on a **secure context**:
`https://`, or `http://localhost`. Reaching the server at a bare LAN address
like `http://192.168.1.20:3000` gives you neither on Android Chrome or iOS
Safari — the app still works, it just can't be installed.

RadioCrate doesn't terminate TLS itself; every installation lives on a different
hostname, so that's the administrator's job. Nothing the frontend serves names a
host — the manifest, the service worker scope and every asset URL are
host-relative — so no configuration is needed on our side whichever route you
take:

- **Home lab or laptop** — a [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/)
  pointed at `http://localhost:<port>`. Certificates are handled for you and
  nothing needs to be exposed to the internet directly.
- **VPS** — [Caddy](https://caddyserver.com/) in front of the binary, which
  provisions and renews Let's Encrypt certificates automatically.

### If you put cookie-based auth in front

Cloudflare Access, Authelia, oauth2-proxy and friends all work, but the manifest
needs one thing the rest of the page doesn't. Browsers fetch
`manifest.webmanifest` with **credentials omitted** by default, so your session
cookie doesn't ride along; the auth layer sees an anonymous request and
redirects to its login page on another origin, which the browser then rejects as
a CORS error. The symptom is no install option at all, and a console error
naming the login host. `index.html` therefore requests the manifest with
`crossorigin="use-credentials"`, which is harmless when nothing is guarding the
app.

Two related things to expect:

- If Chrome still won't offer the install, check the Network tab for the
  **icon** requests. Some auth layers block those too, and Chrome needs at least
  one icon of 144px or larger to install.
- Once the app is cached, a launch with an **expired** auth session shows the UI
  rather than the login page — the service worker serves the shell without
  touching the network, and only the `/api` calls fail. Reload to get bounced to
  the login page properly.

### Installing

- **Android (Chrome)** — the ⋮ menu → *Install app* / *Add to Home screen*.
- **Desktop Linux, Windows, macOS (Chrome/Edge)** — the install icon in the
  address bar, or ⋮ → *Cast, save and share* → *Install page as app*.
- **iOS/iPadOS (Safari)** — Share → *Add to Home Screen*. Safari is the only
  browser on iOS that can do this.

### What works offline

The app shell only: the HTML, the JS/CSS bundle, the icons and the manifest.
That's what makes a cold launch instant, and it means opening the app away from
the server gets you the real UI rather than a browser error page.

Your library and your audio are **not** cached — they're served from `/api`,
which the service worker deliberately never touches (cached query results would
go stale invisibly, and caching audio would break seeking, which relies on range
requests). Without a reachable server the app opens but can't load or play
anything.

### Updating

A new build changes the content hashes in the service worker's Workbox precache
manifest, so the worker re-downloads what changed. It installs in the background
and waits rather than taking over mid-session — a surprise reload during playback
is worse than being one version behind for one launch. To take an update
immediately, run `radiocrate.applyUpdate()` in the devtools console.

### Regenerating the icons

Every icon, favicon and iOS launch image is rasterized from
[branding/logo.svg](branding/logo.svg) into `frontend/public/icons/` (committed,
since the production build embeds them). After changing the logo:

```sh
cargo xtask icons
```

That also rewrites the block of `apple-touch-startup-image` `<link>` tags in
[frontend/index.html](frontend/index.html) (between its `<!-- BEGIN generated:
ios-launch-images -->` / `<!-- END ... -->` markers), which has to name every
supported device explicitly.

The master SVG draws the artwork on a black disc. That disc is positioning
scaffolding, not part of the mark: the generator strips it and paints whatever
ground each destination actually needs — full-bleed black where the platform
masks the icon (Android adaptive, iOS home screen, the boot screen's tile), and
nothing at all where the icon is drawn on someone else's surface. If the logo is
ever redrawn without the disc, `cargo xtask icons` fails loudly rather than
guessing.

The `purpose: "any"` icons ship at every size from 16 up to 512, and the
manifest declares all of them. That is deliberate: Chrome installs a Linux PWA
by writing PNGs into `~/.local/share/icons/hicolor/<N>x<N>/apps/` for a fixed
set of sizes, and anything it can't take from a declared icon it resamples —
which is what made the old icon look soft in the GNOME task switcher.

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
