//! Pure helpers for turning a local file into a storage object key and a public
//! URL. Everything here is deterministic and network-free so it can be unit
//! tested without credentials.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Maximum length of the human-readable slug portion of an object key.
const MAX_SLUG_LEN: usize = 60;

/// Convert an arbitrary file stem into a lowercase, URL- and filesystem-safe
/// slug containing only `[a-z0-9-]`, with no leading/trailing or repeated
/// dashes.
pub fn slugify(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !slug.is_empty() && !prev_dash {
            // Collapse any run of non-alphanumeric characters into a single
            // dash, and never emit a leading dash.
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.len() > MAX_SLUG_LEN {
        slug.truncate(MAX_SLUG_LEN);
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    slug
}

/// First 8 hex characters of the SHA-256 of `bytes`. Used to make object keys
/// unique per content while keeping identical uploads idempotent.
pub fn short_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    hex[..8].to_string()
}

/// Lowercased, sanitized extension of a file name, if any.
///
/// The extension is reduced to ASCII alphanumerics so that no URL-significant
/// character (`?`, `#`, `&`, space, quotes, …) from a crafted or
/// machine-derived file name can leak into the object key or the public URL.
/// Returns `None` if nothing survives sanitization.
fn extension_of(file_name: &str) -> Option<String> {
    Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            e.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect::<String>()
        })
        .filter(|e| !e.is_empty())
}

/// File stem (name without final extension).
fn stem_of(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

/// Build the storage object key:
/// `{prefix}/{date_path}/{slug}-{short_hash}.{ext}`
///
/// `prefix` may be empty or contain slashes; surrounding slashes are trimmed.
/// `date_path` is expected to already be slash-delimited, e.g. `2026/06/13`.
pub fn object_key(prefix: &str, file_name: &str, short_hash: &str, date_path: &str) -> String {
    let slug = slugify(&stem_of(file_name));
    let name = if slug.is_empty() {
        "screenshot".to_string()
    } else {
        slug
    };
    let base = format!("{name}-{short_hash}");
    let file = match extension_of(file_name) {
        Some(ext) => format!("{base}.{ext}"),
        None => base,
    };

    let mut parts: Vec<&str> = Vec::new();
    let trimmed_prefix = prefix.trim_matches('/');
    if !trimmed_prefix.is_empty() {
        parts.push(trimmed_prefix);
    }
    parts.push(date_path);
    parts.push(&file);
    parts.join("/")
}

/// Choose the effective key prefix. A non-empty override wins; an override that
/// is empty or only slashes/whitespace falls back to the configured prefix, so
/// `--prefix ""` (e.g. an unset shell variable) does not silently drop the
/// configured folder.
pub fn effective_prefix<'a>(override_prefix: Option<&'a str>, config_prefix: &'a str) -> &'a str {
    match override_prefix {
        Some(p) if !p.trim_matches('/').trim().is_empty() => p,
        _ => config_prefix,
    }
}

/// Join a public base URL and an object key into a single URL, normalising the
/// slash at the boundary.
pub fn join_public_url(base: &str, key: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        key.trim_start_matches('/')
    )
}

/// Best-effort MIME type for a path based on its extension, defaulting to
/// `application/octet-stream`.
pub fn guess_content_type(path: &Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Login Error"), "login-error");
        // slugify operates on the raw string; extension stripping is object_key's job.
        assert_eq!(
            slugify("Screen Shot 2026-06-13 at 10.45.png"),
            "screen-shot-2026-06-13-at-10-45-png"
        );
        // object_key, in contrast, strips the extension before slugifying the stem.
        assert_eq!(
            object_key(
                "",
                "Screen Shot 2026-06-13 at 10.45.png",
                "abcd1234",
                "2026/06/13"
            ),
            "2026/06/13/screen-shot-2026-06-13-at-10-45-abcd1234.png"
        );
    }

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(slugify("  --Hello___World!!  "), "hello-world");
        assert_eq!(slugify("###"), "");
        assert_eq!(slugify("a@@@b"), "a-b");
    }

    #[test]
    fn slugify_truncates() {
        let long = "a".repeat(200);
        assert_eq!(slugify(&long).len(), MAX_SLUG_LEN);
    }

    #[test]
    fn short_hash_of_empty() {
        // SHA-256("") = e3b0c44298fc1c14...
        assert_eq!(short_hash(b""), "e3b0c442");
    }

    #[test]
    fn short_hash_is_stable() {
        assert_eq!(short_hash(b"hello"), short_hash(b"hello"));
        assert_ne!(short_hash(b"hello"), short_hash(b"world"));
    }

    #[test]
    fn object_key_with_prefix() {
        let key = object_key("screenshots", "Login Error.png", "deadbeef", "2026/06/13");
        assert_eq!(key, "screenshots/2026/06/13/login-error-deadbeef.png");
    }

    #[test]
    fn object_key_without_prefix() {
        let key = object_key("", "shot.PNG", "0badf00d", "2026/06/13");
        assert_eq!(key, "2026/06/13/shot-0badf00d.png");
    }

    #[test]
    fn object_key_trims_prefix_slashes() {
        let key = object_key("/a/b/", "x.jpg", "12345678", "2026/01/02");
        assert_eq!(key, "a/b/2026/01/02/x-12345678.jpg");
    }

    #[test]
    fn object_key_no_extension() {
        let key = object_key("", "README", "12345678", "2026/01/02");
        assert_eq!(key, "2026/01/02/readme-12345678");
    }

    #[test]
    fn object_key_unnamed_falls_back() {
        let key = object_key("", "###.png", "12345678", "2026/01/02");
        assert_eq!(key, "2026/01/02/screenshot-12345678.png");
    }

    #[test]
    fn join_public_url_normalises_slashes() {
        assert_eq!(
            join_public_url("https://pub-x.r2.dev/", "/a/b.png"),
            "https://pub-x.r2.dev/a/b.png"
        );
        assert_eq!(
            join_public_url("https://cdn.example.com", "a/b.png"),
            "https://cdn.example.com/a/b.png"
        );
    }

    #[test]
    fn content_type_for_png() {
        assert_eq!(guess_content_type(Path::new("a/b.png")), "image/png");
        assert_eq!(guess_content_type(Path::new("a/b.jpg")), "image/jpeg");
    }

    #[test]
    fn object_key_sanitizes_extension() {
        // URL-significant characters in the extension are stripped, keeping the
        // key (and therefore the public URL) composed only of safe characters.
        assert_eq!(
            object_key("", "shot.png?evil", "12345678", "2026/06/13"),
            "2026/06/13/shot-12345678.pngevil"
        );
        assert_eq!(
            object_key("", "a.j peg", "12345678", "2026/06/13"),
            "2026/06/13/a-12345678.jpeg"
        );
        // An extension made entirely of symbols disappears, yielding no extension.
        assert_eq!(
            object_key("", "a.<>", "12345678", "2026/06/13"),
            "2026/06/13/a-12345678"
        );
    }

    #[test]
    fn effective_prefix_falls_back_on_empty_override() {
        assert_eq!(
            effective_prefix(Some("bugs/123"), "screenshots"),
            "bugs/123"
        );
        assert_eq!(effective_prefix(Some(""), "screenshots"), "screenshots");
        assert_eq!(effective_prefix(Some("/"), "screenshots"), "screenshots");
        assert_eq!(effective_prefix(Some("   "), "screenshots"), "screenshots");
        assert_eq!(effective_prefix(None, "screenshots"), "screenshots");
        assert_eq!(effective_prefix(None, ""), "");
    }
}
