//! smenago — upload screenshots to S3-compatible storage and print a public link.

mod config;
mod naming;
mod uploader;

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

/// Upload screenshots to S3-compatible storage (Cloudflare R2 & friends) and
/// print a shareable public URL.
#[derive(Parser)]
#[command(
    name = "smenago",
    version,
    about,
    long_about = None,
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Path to the config file (default: ~/.config/smenago/config.json).
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Files to upload when no subcommand is given (shorthand for `upload`).
    #[command(flatten)]
    upload: UploadArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args, Clone)]
struct UploadArgs {
    /// Screenshot file(s) to upload.
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Print only the resulting URL(s); suppress status output.
    #[arg(short, long)]
    quiet: bool,

    /// Print results as Markdown image syntax: `![name](url)`.
    #[arg(short, long, conflicts_with = "json")]
    markdown: bool,

    /// Print results as a JSON array.
    #[arg(long)]
    json: bool,

    /// Override the configured key prefix for this run.
    #[arg(long, value_name = "PREFIX")]
    prefix: Option<String>,

    /// Compute the object key and URL without uploading (no credentials used).
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Upload screenshot file(s) and print their public URL(s).
    Upload(UploadArgs),

    /// Manage the smenago configuration file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Generate a shell completion script for the given shell.
    Completions {
        /// Target shell (bash, zsh, fish, elvish, powershell).
        #[arg(value_name = "SHELL")]
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Write a template config file (refuses to overwrite without --force).
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
    /// Print the resolved config file path.
    Path,
    /// Print the active configuration (credentials redacted).
    Show,
}

/// Per-file result, retained so structured output can report failures too.
enum FileReport {
    Ok(uploader::UploadOutcome),
    Failed { file: PathBuf, message: String },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let config_path = config::resolve_config_path(cli.config.as_deref());

    match cli.command {
        None => run_upload(&config_path, &cli.upload).await,
        Some(Command::Upload(args)) => run_upload(&config_path, &args).await,
        Some(Command::Config { action }) => run_config(action, &config_path),
        Some(Command::Completions { shell }) => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut io::stdout());
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn run_upload(config_path: &Path, args: &UploadArgs) -> Result<ExitCode> {
    if args.files.is_empty() {
        bail!("no files given. Usage: smenago <FILE>...  (try `smenago --help`)");
    }

    let cfg = config::Config::load(config_path)?;
    let client = if args.dry_run {
        None
    } else {
        Some(uploader::build_client(&cfg).await?)
    };

    let mut reports = Vec::with_capacity(args.files.len());
    let mut failures = 0usize;

    for file in &args.files {
        match uploader::upload(client.as_ref(), &cfg, file, args.prefix.as_deref()).await {
            Ok(outcome) => reports.push(FileReport::Ok(outcome)),
            Err(err) => {
                // upload() error messages are self-contained (they include the
                // path), so don't prefix the path again here.
                let message = format!("{err:#}");
                eprintln!("error: {message}");
                reports.push(FileReport::Failed {
                    file: file.clone(),
                    message,
                });
                failures += 1;
            }
        }
    }

    print_reports(&reports, args)?;

    if failures > 0 {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn print_reports(reports: &[FileReport], args: &UploadArgs) -> Result<()> {
    if args.json {
        // The JSON array has one entry per input file (in order), with `error`
        // set on failures, so it is a complete, machine-checkable record.
        let arr: Vec<_> = reports
            .iter()
            .map(|r| match r {
                FileReport::Ok(o) => serde_json::json!({
                    "file": o.file,
                    "key": o.key,
                    "url": o.url,
                    "bytes": o.bytes,
                    "uploaded": o.uploaded,
                    "error": serde_json::Value::Null,
                }),
                FileReport::Failed { file, message } => serde_json::json!({
                    "file": file,
                    "key": serde_json::Value::Null,
                    "url": serde_json::Value::Null,
                    "bytes": serde_json::Value::Null,
                    "uploaded": false,
                    "error": message,
                }),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    // Plain / markdown / quiet: only successful URLs go to stdout (so piping
    // captures usable links); failures were already reported to stderr.
    for report in reports {
        let FileReport::Ok(o) = report else { continue };
        if args.markdown {
            let alt = o
                .file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("screenshot");
            println!("![{alt}]({})", o.url);
        } else {
            println!("{}", o.url);
        }

        if !args.quiet {
            let tag = if o.uploaded { "uploaded" } else { "dry-run " };
            eprintln!("  {tag} {} ({})", o.file.display(), human_size(o.bytes));
        }
    }
    Ok(())
}

fn run_config(action: ConfigAction, config_path: &Path) -> Result<ExitCode> {
    match action {
        ConfigAction::Path => {
            println!("{}", config_path.display());
        }
        ConfigAction::Init { force } => {
            config::write_template(config_path, force)?;
            eprintln!("Wrote template config to {}", config_path.display());
            eprintln!("Next: edit it with your S3/R2 credentials, then run `smenago <file>`.");
        }
        ConfigAction::Show => {
            let mut cfg = config::Config::load(config_path)?;
            cfg.access_key_id = redact(&cfg.access_key_id);
            cfg.secret_access_key = redact(&cfg.secret_access_key);
            println!("{}", serde_json::to_string_pretty(&cfg)?);
            eprintln!("(config: {})", config_path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Redact a secret, keeping a short non-sensitive prefix as a sanity check.
fn redact(secret: &str) -> String {
    let prefix: String = secret.chars().take(4).collect();
    if secret.chars().count() <= 4 {
        "****".to_string()
    } else {
        format!("{prefix}…(redacted)")
    }
}

/// Human-friendly byte size, e.g. `24.3 KB`.
fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        // Catches clap derive misconfiguration at test time.
        Cli::command().debug_assert();
    }

    #[test]
    fn redact_short_and_long() {
        assert_eq!(redact("abc"), "****");
        assert_eq!(redact("abcdefgh"), "abcd…(redacted)");
    }

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }
}
