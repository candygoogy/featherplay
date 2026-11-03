## What and why

<!-- What changes for someone using featherplay, and why. Link the issue if there is one. -->

## Checks

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `RUSTFLAGS="--cfg ci" cargo nextest run --workspace` and `cargo test --workspace --doc`
- [ ] For UI changes: before and after snapshots (`--snapshot <page> --fixtures`), dark and light
- [ ] Within scope: audio only, no downloads, no telemetry (CONTRIBUTING.md)
