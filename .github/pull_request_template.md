## What and why

<!-- What changes for someone using featherplay, and why. Link the issue if there is one. -->

## Checks

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `RUSTFLAGS="--cfg ci" cargo nextest run --workspace` and `RUSTFLAGS="--cfg ci" cargo test --workspace --doc`
- [ ] For new or changed strings: marked for translation, and `apps/featherplay/po/update.sh` run
- [ ] For UI changes: before and after snapshots (`--snapshot <page> --fixtures`), dark and light
- [ ] Within scope: audio only, no downloads, no telemetry (CONTRIBUTING.md)
