# Collectune

A client-server app for managing and playing your personal collection of music files.

## Code Formatting

Uses `rustfmt` with project-specific settings in [rustfmt.toml](rustfmt.toml).

```
cargo fmt
```

To check formatting without modifying files:

```
cargo fmt --check
```

## Linting

Uses `clippy` with workspace-level lint rules defined in [Cargo.toml](Cargo.toml).

```
cargo clippy
```
