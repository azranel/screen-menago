//! Configuration loading for smenago.
//!
//! The config file lives at `~/.config/smenago/config.json` by default. The
//! location can be overridden with `--config <path>` or the `SMENAGO_CONFIG`
//! environment variable. It holds S3/R2 credentials, so `config init` writes it
//! with `0600` permissions.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Environment variable that overrides the config file location.
pub const CONFIG_ENV: &str = "SMENAGO_CONFIG";

/// Template written by `smenago config init`.
pub const TEMPLATE: &str = r#"{
  "account_id": "YOUR_CLOUDFLARE_ACCOUNT_ID",
  "bucket": "your-bucket-name",
  "access_key_id": "YOUR_R2_ACCESS_KEY_ID",
  "secret_access_key": "YOUR_R2_SECRET_ACCESS_KEY",
  "public_url_base": "https://pub-xxxxxxxx.r2.dev",
  "key_prefix": "screenshots",
  "region": "auto"
}
"#;

fn default_region() -> String {
    "auto".to_string()
}

/// On-disk configuration.
///
/// Note: `Debug` is implemented manually to redact credentials, so an
/// accidental `{cfg:?}` in a future log line or error context cannot leak the
/// secret key. Do not add `Debug` to the derive list.
#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    /// Cloudflare account id. When set (and `endpoint` is not), the S3 endpoint
    /// is derived as `https://<account_id>.r2.cloudflarestorage.com`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,

    /// Full S3 API endpoint. Takes precedence over `account_id`. Set this
    /// directly for non-R2 S3-compatible providers (MinIO, Spaces, B2, AWS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Signing region. R2 expects `auto`; AWS S3 expects e.g. `us-east-1`.
    #[serde(default = "default_region")]
    pub region: String,

    /// Bucket to upload into.
    pub bucket: String,

    /// S3 access key id.
    pub access_key_id: String,

    /// S3 secret access key.
    pub secret_access_key: String,

    /// Public base URL used to build the returned link, e.g. the r2.dev
    /// development URL or a connected custom domain. This is a *different*
    /// host from `endpoint` — uploads go to `endpoint`, links point here.
    pub public_url_base: String,

    /// Optional key prefix (folder) prepended to every object key.
    #[serde(default)]
    pub key_prefix: String,

    /// Force path-style addressing (needed for some providers such as MinIO).
    #[serde(default)]
    pub force_path_style: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("account_id", &self.account_id)
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &"<redacted>")
            .field("public_url_base", &self.public_url_base)
            .field("key_prefix", &self.key_prefix)
            .field("force_path_style", &self.force_path_style)
            .finish()
    }
}

impl Config {
    /// Load and validate the config from `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "no config found at {}\n\nRun `smenago config init` to create one, \
                     then edit it with your S3/R2 credentials.",
                    path.display()
                )
            } else {
                anyhow!("failed to read config at {}: {e}", path.display())
            }
        })?;

        let cfg: Config = serde_json::from_str(&data).with_context(|| {
            format!(
                "failed to parse config at {} (invalid JSON?)",
                path.display()
            )
        })?;
        cfg.validate(path)?;
        Ok(cfg)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.access_key_id.starts_with("YOUR_")
            || self.secret_access_key.starts_with("YOUR_")
            || self.public_url_base.contains("xxxxxxxx")
            || self.bucket.starts_with("your-")
        {
            bail!(
                "config at {} still contains placeholder values — edit it with your \
                 real S3/R2 credentials",
                path.display()
            );
        }
        if self.bucket.trim().is_empty() {
            bail!("config `bucket` must not be empty");
        }
        if self.public_url_base.trim().is_empty() {
            bail!("config `public_url_base` must not be empty (set it to your r2.dev or custom domain)");
        }
        // resolved_endpoint enforces endpoint/account_id presence.
        self.resolved_endpoint()?;
        Ok(())
    }

    /// The S3 API endpoint to upload to, derived from `account_id` if not set
    /// explicitly.
    pub fn resolved_endpoint(&self) -> Result<String> {
        if let Some(ep) = self.endpoint.as_deref() {
            if !ep.trim().is_empty() {
                return Ok(ep.trim().to_string());
            }
        }
        if let Some(id) = self.account_id.as_deref() {
            if !id.trim().is_empty() {
                return Ok(format!("https://{}.r2.cloudflarestorage.com", id.trim()));
            }
        }
        bail!("config must set either `endpoint` (full S3 URL) or `account_id` (for Cloudflare R2)")
    }
}

/// Resolve the config file path: `--config` override, then `$SMENAGO_CONFIG`,
/// then `$XDG_CONFIG_HOME/smenago/config.json`, then `~/.config/smenago/config.json`.
pub fn resolve_config_path(override_path: Option<&Path>) -> PathBuf {
    if let Some(p) = override_path {
        return p.to_path_buf();
    }
    if let Ok(p) = std::env::var(CONFIG_ENV) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    config_dir().join("config.json")
}

/// Config directory. Honours `XDG_CONFIG_HOME`, otherwise `~/.config/smenago`
/// (note: deliberately *not* `dirs::config_dir()`, which is
/// `~/Library/Application Support` on macOS — the tool standardises on
/// `~/.config` across platforms).
pub fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("smenago");
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("smenago")
}

/// Write the template config to `path`, creating parent directories. Refuses to
/// overwrite an existing file unless `force` is set. Sets `0600` on unix.
pub fn write_template(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "config already exists at {} (use `--force` to overwrite)",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    write_secure(path, TEMPLATE)?;
    // Belt-and-suspenders: `mode` on create does not apply when overwriting an
    // existing file, so also chmod explicitly to cover the `--force` path.
    set_permissions_600(path)?;
    Ok(())
}

/// Create (or truncate) the file and write `contents`. On unix the file is
/// created atomically with mode `0600` so there is no window where it is
/// group/world-readable, regardless of umask.
#[cfg(unix)]
fn write_secure(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(not(unix))]
fn write_secure(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(unix)]
fn set_permissions_600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_permissions_600(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_r2_endpoint_from_account_id() {
        let cfg = Config {
            account_id: Some("abc123".into()),
            endpoint: None,
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            public_url_base: "https://pub-x.r2.dev".into(),
            key_prefix: String::new(),
            force_path_style: false,
        };
        assert_eq!(
            cfg.resolved_endpoint().unwrap(),
            "https://abc123.r2.cloudflarestorage.com"
        );
    }

    #[test]
    fn explicit_endpoint_wins() {
        let cfg = Config {
            account_id: Some("abc123".into()),
            endpoint: Some("https://minio.local:9000".into()),
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            public_url_base: "https://cdn.example.com".into(),
            key_prefix: String::new(),
            force_path_style: true,
        };
        assert_eq!(cfg.resolved_endpoint().unwrap(), "https://minio.local:9000");
    }

    #[test]
    fn endpoint_required() {
        let cfg = Config {
            account_id: None,
            endpoint: None,
            region: "auto".into(),
            bucket: "b".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            public_url_base: "https://cdn.example.com".into(),
            key_prefix: String::new(),
            force_path_style: false,
        };
        assert!(cfg.resolved_endpoint().is_err());
    }

    #[test]
    fn template_parses_but_is_rejected_as_placeholder() {
        let cfg: Config = serde_json::from_str(TEMPLATE).unwrap();
        assert!(cfg.validate(Path::new("/tmp/x.json")).is_err());
    }
}
