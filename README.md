# smenago

A tiny CLI that uploads a screenshot (or any file) to S3-compatible object
storage and prints back a public URL. Built for AI coding agents that take
browser-testing screenshots and need to drop a link into an issue or PR
description — without committing binaries to a throwaway branch.

Defaults are tuned for **Cloudflare R2**, but it works with any S3-compatible
provider (AWS S3, MinIO, DigitalOcean Spaces, Backblaze B2, …).

```console
$ smenago login-error.png
https://pub-3f9a2b1c.r2.dev/screenshots/2026/06/13/login-error-3f9a2b1c.png
  uploaded login-error.png (48.2 KB)
```

The URL goes to **stdout**; status goes to **stderr** — so piping captures
exactly the link:

```console
$ URL=$(smenago -q shot.png) && echo "$URL"
```

## Install

### Homebrew (recommended)

`smenago` is built from source by Homebrew, so it needs no prebuilt bottles.
The formula lives in this repo, so tap it by URL once, then install:

```console
brew tap azranel/screen-menago https://github.com/azranel/screen-menago
brew install azranel/screen-menago/smenago
```

Upgrade later with `brew update && brew upgrade smenago`.

> Why the long `brew tap` line? Homebrew's short `brew tap user/name` syntax
> only works for repos named `homebrew-<name>`. This formula is co-hosted in the
> application repo, so the two-argument form (which accepts any repo URL) is
> used instead.

### From source with Cargo

```console
git clone https://github.com/azranel/screen-menago
cd screen-menago
cargo install --path .
```

### Testing the Homebrew formula locally

To iterate on your own code, use `cargo install --path .` (above) — it builds
your working tree. The Homebrew formula instead builds the **released tag** in
its `url` stanza, so to test the formula itself against a tag:

```console
HOMEBREW_NO_INSTALL_FROM_API=1 brew install --build-from-source ./Formula/smenago.rb
```

(`--build-from-source` only means "compile instead of using a bottle"; Homebrew
still fetches the tagged source and checks its `sha256`, so this needs the
formula's `sha256` to be filled in for a real release.) To build the latest
`main` instead of a tag: `brew install --HEAD azranel/screen-menago/smenago`.

## Configure

`smenago` reads `~/.config/smenago/config.json` (override with `--config <path>`
or `$SMENAGO_CONFIG`). Create a template and edit it:

```console
$ smenago config init
Wrote template config to /Users/you/.config/smenago/config.json
Next: edit it with your S3/R2 credentials, then run `smenago <file>`.
```

```jsonc
{
  // Cloudflare account id — the upload endpoint is derived as
  // https://<account_id>.r2.cloudflarestorage.com.
  // (For non-R2 providers, set "endpoint" instead — see below.)
  "account_id": "your-cloudflare-account-id",

  "bucket": "screenshots",
  "access_key_id": "<R2 access key id>",
  "secret_access_key": "<R2 secret access key>",

  // Public base used to build the returned link. This is a DIFFERENT host
  // from the upload endpoint: it's your bucket's r2.dev dev URL or a
  // connected custom domain.
  "public_url_base": "https://pub-xxxxxxxx.r2.dev",

  // Optional: folder prefix for every object (default: none).
  "key_prefix": "screenshots",

  // Optional: signing region (default: "auto", which is correct for R2).
  "region": "auto"
}
```

The file is created with `0600` permissions because it holds your secret key.

### Cloudflare R2 setup, end to end

1. **Create a bucket** in the Cloudflare dashboard (R2 → Create bucket).
2. **Create an API token** (R2 → Manage R2 API Tokens → *Object Read & Write*).
   Copy the **Access Key ID** and **Secret Access Key** into the config.
3. **Find your account id** (R2 overview page, right sidebar) → `account_id`.
4. **Enable public access** so the links resolve without credentials:
   - **Quick / dev:** bucket → Settings → *Public Development URL* → enable.
     You get `https://pub-<hash>.r2.dev`. Put that in `public_url_base`.
     (Rate-limited and uncached — fine for issue screenshots, not for prod.)
   - **Production:** bucket → Settings → *Custom Domains* → connect a domain
     you already manage in Cloudflare, e.g. `https://shots.example.com`, and
     use that as `public_url_base`.

> R2 does **not** support per-object ACLs, so `smenago` never sends one —
> public access is controlled entirely by the bucket setting above. Anything in
> a public bucket is readable by anyone who has the key (URL), so use a bucket
> dedicated to shareable screenshots.

> **Heads-up:** `smenago` confirms the *upload* succeeded, but it does not
> verify the printed link is publicly reachable. If you skip the public-access
> step above (or `public_url_base` points at the wrong host/bucket), uploads
> still succeed and a URL is printed — but it will 404 for anyone opening it.
> Verify public access once during setup, e.g. open a `--dry-run`-style link in
> a browser after your first real upload.

### Other S3-compatible providers

Set `endpoint` explicitly instead of `account_id`, and adjust `region`:

```jsonc
{
  "endpoint": "https://nyc3.digitaloceanspaces.com",
  "region": "nyc3",
  "bucket": "my-space",
  "access_key_id": "...",
  "secret_access_key": "...",
  "public_url_base": "https://my-space.nyc3.cdn.digitaloceanspaces.com",
  "force_path_style": false
}
```

For MinIO and similar, set `"force_path_style": true`.

## Usage

```console
smenago <FILE>...                 # upload one or more files, print URL(s)
smenago -q shot.png               # quiet: print only the URL
smenago -m shot.png               # Markdown: ![shot](https://…)
smenago --json a.png b.png        # JSON array of {file, key, url, bytes}
smenago --dry-run shot.png        # compute the URL without uploading
smenago --prefix bugs/123 shot.png  # override the key prefix for this run

smenago config init [--force]     # write the template config
smenago config path               # print the resolved config path
smenago config show               # print config (secret redacted)
smenago completions zsh           # print a shell completion script
```

`smenago <file>` is shorthand for `smenago upload <file>`.

**Output contract.** Successful URLs go to **stdout** (one per line, or Markdown
with `-m`); status and per-file errors go to **stderr**. With multiple files,
**always check the exit code** — it is non-zero if any file failed. In `--json`
mode the array contains one object per input file (in order) with an `error`
field set on failures, so it is a complete, machine-checkable record; plain and
Markdown output print only the successes.

### Object keys

Uploaded objects are keyed as:

```
{key_prefix}/{YYYY}/{MM}/{DD}/{slug}-{hash8}.{ext}
```

where `slug` is the sanitized file name (ASCII `[a-z0-9-]`; names with no ASCII
letters fall back to `screenshot`) and `hash8` is the first 8 hex of the file's
SHA-256. Dates are **UTC**. Re-uploading the same file on the same UTC day is
idempotent (same key → same URL); across a UTC day boundary the date — and thus
the link — changes.

### For coding agents

Drop a link straight into a Markdown description:

```console
smenago -m screenshots/login-error.png
# => ![login-error](https://pub-….r2.dev/screenshots/2026/06/13/login-error-….png)
```

Or capture the bare URL for templating:

```console
url=$(smenago -q screenshots/login-error.png)
```

## Development

This repo uses [mise](https://mise.jdx.dev) to manage the Rust toolchain
(`mise.toml`). With mise activated:

```console
cargo test                                # unit tests (no network needed)
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

See [CLAUDE.md](CLAUDE.md) for an architecture overview.

## License

MIT — see [LICENSE](LICENSE).
