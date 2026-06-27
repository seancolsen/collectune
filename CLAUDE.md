## Validation

Check your uncommitted changes to see whether you've modified any `Cargo.toml`
or `Cargo.lock` files, and if so, *which* ones.

### When a build is expensive (don't run cargo yourself)

If you've modified the **top-level `Cargo.toml`** or **`backend/Cargo.toml`**,
do not run any cargo commands yourself — not even `cargo check`. Changing
dependencies at these levels can force a rebuild of `duckdb-sys`, which can take
over 20 minutes. Stop your work and prompt me to run the cargo commands myself.

### Otherwise (builds are cheap — run cargo yourself)

If you've made no Cargo changes, or only changed Cargo files *outside* the
backend (e.g. `frontend/Cargo.toml`), then go ahead and run cargo yourself.
These builds don't touch `duckdb-sys`, so they're fast. Run, fixing any errors
you notice:

1. `cargo check`
2. `cargo clippy`
3. `cargo fmt`

You may also run the **frontend snapshot tests** (`cargo test -p frontend`) to
generate and inspect widget snapshots — this is part of the front-end
self-validation workflow and only builds the frontend crate.

**Do not ever run `cargo build`** (I run release/WASM builds myself), and don't
run the full `cargo test` across the workspace — scope test runs to the crate
you're working on (e.g. `-p frontend`).

## Inspecting widget snapshots

The frontend snapshot tests render widgets to PNGs under `frontend/tests/snapshots/`.

When I refer to one of these images in a prompt, analyze the image yourself to understand my prompt within the context of that UI before doing any work. Then, after you make your code changes, validate your work by generating new images and analyzing them. Use your best judgement to determine whether the resulting images fulfil the requirements set fort in the prompt.

When you need to *look at* or *measure* one, don't fumble with `convert txt:` dumps or assume Pillow is missing — the container has tooling for this (see DEVELOPMENT.md, "Inspecting & measuring a failure"):

- **What changed?** A failure leaves `<name>.diff.png` next to the baseline — open that first. `scripts/snapshot_composite.sh <baseline.png>` stitches `old | diff | new` into one strip.

- **Measure to the pixel** (margins, symmetry, boundaries): `scripts/measure_snapshot.py <png> [--row N] [--col N]`. It bakes in the conventions — **PPP = 2** (images are 2× logical size), an **8 logical-px harness margin** around the cropped content, resting border gray **`#C8C8C8`** — so you don't re-derive them, and classifies pixels as background/border/fill/content.

- `python3` has **Pillow + NumPy** preinstalled, so `from PIL import Image` works if you'd rather read pixels directly.

