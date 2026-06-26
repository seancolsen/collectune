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
