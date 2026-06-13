//! S3/R2 client construction and the upload itself.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::config::{Credentials, RequestChecksumCalculation, ResponseChecksumValidation};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use chrono::Utc;

use crate::config::Config;
use crate::naming;

/// Upper bound on the file size smenago will read into memory and upload.
/// Screenshots are far smaller; this guards against accidentally streaming a
/// huge file (or a never-ending device file) into RAM.
const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

/// Result of planning/performing an upload.
pub struct UploadOutcome {
    pub file: PathBuf,
    pub key: String,
    pub url: String,
    pub bytes: usize,
    /// `false` when this was a dry run (no network call made).
    pub uploaded: bool,
}

/// Build an S3 client pointed at the configured endpoint.
///
/// Crucially, both checksum settings are forced to `WhenRequired`. As of
/// aws-sdk-s3 1.69.0 (Jan 2025) the SDK adds CRC32 integrity headers to every
/// PutObject by default, which Cloudflare R2 (and several other S3-compatible
/// providers) reject with HTTP 400. `WhenRequired` suppresses them.
pub async fn build_client(cfg: &Config) -> Result<Client> {
    let endpoint = cfg.resolved_endpoint()?;
    let creds = Credentials::new(
        cfg.access_key_id.clone(),
        cfg.secret_access_key.clone(),
        None,
        None,
        "smenago",
    );

    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .endpoint_url(endpoint)
        .region(Region::new(cfg.region.clone()))
        .credentials_provider(creds)
        .load()
        .await;

    let mut builder = aws_sdk_s3::config::Builder::from(&sdk_config)
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired);
    if cfg.force_path_style {
        builder = builder.force_path_style(true);
    }

    Ok(Client::from_conf(builder.build()))
}

/// Upload a single file. When `client` is `None`, this is a dry run: the object
/// key and public URL are computed (and the file is read, to hash it and report
/// its size) but no network call is made.
pub async fn upload(
    client: Option<&Client>,
    cfg: &Config,
    path: &Path,
    prefix_override: Option<&str>,
) -> Result<UploadOutcome> {
    let meta = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("failed to stat file {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    if meta.len() > MAX_FILE_BYTES {
        bail!(
            "{} is too large ({} bytes); smenago refuses files larger than {} MB",
            path.display(),
            meta.len(),
            MAX_FILE_BYTES / (1024 * 1024)
        );
    }

    let data = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read file {}", path.display()))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("path {} has no valid file name", path.display()))?;

    let hash = naming::short_hash(&data);
    let date_path = Utc::now().format("%Y/%m/%d").to_string();
    let prefix = naming::effective_prefix(prefix_override, cfg.key_prefix.as_str());
    let key = naming::object_key(prefix, file_name, &hash, &date_path);
    let url = naming::join_public_url(&cfg.public_url_base, &key);
    let content_type = naming::guess_content_type(path);
    let bytes = data.len();

    let uploaded = if let Some(client) = client {
        client
            .put_object()
            .bucket(&cfg.bucket)
            .key(&key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to upload {} to bucket '{}' (key '{}')",
                    path.display(),
                    cfg.bucket,
                    key
                )
            })?;
        true
    } else {
        false
    };

    Ok(UploadOutcome {
        file: path.to_path_buf(),
        key,
        url,
        bytes,
        uploaded,
    })
}
