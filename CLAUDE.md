## Validation

Check your uncomitted changes to see whether you've modified any `Cargo.toml` or `Cargo.lock` files.

If you've modified Cargo files, then do not run any cargo commands yourself, not even `cargo check`. These commands can take many minutes to run. Stop your work and prompt me to run the cargo commands myself.

If you've not modified any Cargo files, then run the following cargo commands in this order, fixing any errors you notice:

1. `cargo check`
2. `cargo clippy`
3. `cargo fmt`

**Do not ever run `cargo build`**. I will run this myself. And we don't have tests yet, so don't try to run `cargo test` either.

