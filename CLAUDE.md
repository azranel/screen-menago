# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

`smenago` is a small Rust CLI that uploads a file (typically a screenshot) to
S3-compatible object storage and prints a public URL. It exists so coding agents
can put screenshot links into issue/PR descriptions instead of committing
binaries to throwaway branches. Defaults target **Cloudflare R2**; any
S3-compatible provider works via explicit `endpoint`.

The shipped binary is `smenago`.

## Toolchain

This repo uses [mise](https://mise.jdx.dev) (`mise.toml`) to pin Rust. If `cargo`
isn't on `PATH`, prefix commands with `mise exec --`:

```bash
mise exec -- cargo test
```

## Common commands

```bash
cargo test                              # unit tests — no network/credentials needed
cargo clippy --all-targets -- -D warnings   # lint (CI treats warnings as errors)
cargo fmt --all                         # format (CI runs `cargo fmt --all --check`)
cargo build --release                   # release binary at target/release/smenago

# Exercise the binary without uploading:
cargo run -- --dry-run path/to/shot.png
```

## Architecture

Four modules, each small and single-purpose:

- `src/main.rs` — clap CLI definition, subcommand dispatch, and output
  formatting (plain URL / `-q` / `-m` markdown / `--json`). `main` returns
  `ExitCode`; errors are printed via anyhow's `{:?}` for a full cause chain.
- `src/config.rs` — `Config` struct, JSON loading + validation, config-path
  resolution, and `config init` template writing (with `0600` perms).
- `src/uploader.rs` — builds the `aws-sdk-s3` client and performs the upload.
- `src/naming.rs` — **pure, network-free** helpers (slug, content hash, object
  key, public-URL join, content-type). All unit tests that don't need a network
  live here and in `config.rs`.

Flow: `main` resolves the config path → `Config::load` → `uploader::build_client`
→ `uploader::upload` (computes key+URL via `naming`, then `PutObject`).

## Non-obvious constraints (don't regress these)

- **R2 checksum fix.** `uploader::build_client` sets both
  `request_checksum_calculation` and `response_checksum_validation` to
  `WhenRequired`. aws-sdk-s3 ≥ 1.69.0 otherwise sends CRC32 integrity headers on
  every PutObject, which Cloudflare R2 rejects with HTTP 400. Do not remove these.
- **No ACLs.** R2 doesn't implement object ACLs; public access is a bucket-level
  setting. Never add `.acl(...)` to the PutObject call.
- **Upload host ≠ public host.** `endpoint`/`account_id` is the authenticated S3
  API host; `public_url_base` (r2.dev or custom domain) is where links point.
  They are always different and both are required.
- **Config path is `~/.config`, not `dirs::config_dir()`.** On macOS the latter
  is `~/Library/Application Support`; we deliberately standardise on `~/.config`
  (honouring `XDG_CONFIG_HOME`). See `config::config_dir`.
- **Config holds secrets** → created atomically with mode `0600` (unix);
  `config show` redacts both `access_key_id` and `secret_access_key`; `Config`
  has a hand-written `Debug` that redacts credentials (don't re-derive `Debug`).
- **Object keys are partitioned by UTC date**; same-file/same-UTC-day uploads
  are idempotent. The slug and the file extension are both sanitized to ASCII
  `[a-z0-9-]` / `[a-z0-9]` so nothing URL-breaking reaches the key or URL.
- **Uploads are bounded**: non-regular files (FIFOs, devices) are rejected and
  files over `MAX_FILE_BYTES` (100 MB) are refused before reading into memory.
- **Structured output is complete**: `--json` includes failed files with an
  `error` field; plain/markdown print only successes (failures → stderr, and
  the exit code is non-zero).
- `Cargo.lock` is committed (the binary is distributed via a Homebrew formula
  that builds with `cargo install --locked`; CI also runs `--locked`).

## Tests

Tests are pure and offline: object-key/slug/URL logic in `naming.rs`, endpoint
derivation + placeholder rejection in `config.rs`, and a clap `debug_assert`
plus output-helper tests in `main.rs`. Real uploads are not covered by tests;
verify those manually with a configured bucket or `--dry-run`.

## Release / distribution

Homebrew formula is `Formula/smenago.rb` (builds from source). Release flow:

1. Bump `version` in `Cargo.toml`, commit.
2. Tag `vX.Y.Z` and push the tag (a GitHub release auto-creates the source
   tarball).
3. `shasum -a 256` the tag tarball and update `url` + `sha256` in the formula.
4. Commit the formula update on `main`.

Install path for users: `brew tap azranel/screen-menago <repo-url>` then
`brew install azranel/screen-menago/smenago` (see README).
