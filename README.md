<!-- Copyright (c) 2026 Michael Grier -->
# worrywart-rs

[![CI](https://github.com/MikeGrier/worrywart-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/MikeGrier/worrywart-rs/actions/workflows/ci.yml)
[![release-please](https://github.com/MikeGrier/worrywart-rs/actions/workflows/release-please.yml/badge.svg)](https://github.com/MikeGrier/worrywart-rs/actions/workflows/release-please.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A library that worries, maybe too much, about child processes

## Crates

| Crate | What it is |
|---|---|
| [`worrywart`](crates/worrywart) | Core library crate. |


## Build

Requires a recent Rust toolchain (MSRV: see `[workspace.package].rust-version`
in [Cargo.toml](Cargo.toml)).

```powershell
cargo build --workspace --release
cargo test --workspace
```

## Release pipeline

Versioning, tagging, and publishing are automated:

1. Land commits on `main` using
   [Conventional Commits](https://www.conventionalcommits.org/)
   (`fix:`, `feat:`, `feat!:`).
2. [`release-please`](.github/workflows/release-please.yml) opens or updates
   a Release PR that bumps the workspace version and the changelog.
3. Merging the Release PR creates a `v<version>` tag and publishes the crate
   to [crates.io](https://crates.io) automatically.

## License

MIT — see [LICENSE](LICENSE).
