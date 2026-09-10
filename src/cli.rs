//! Command-line interface definitions and parsing.

use clap::Parser;
use std::path::PathBuf;

/// RDNS - Modern, Lightweight, Webhook-driven DDNS Client
#[derive(Parser, Debug, Clone)]
#[command(name = "rdns", version, about, long_about = None)]
pub struct Cli {
    /// Path to YAML configuration file
    #[arg(short = 'c', long = "config", default_value = "config.yaml")]
    pub config: PathBuf,

    /// Single execution mode: update once and immediately exit
    #[arg(long = "once", conflicts_with = "daemon")]
    pub once: bool,

    /// Long-running daemon mode with asynchronous interval polling
    #[arg(long = "daemon", conflicts_with = "once")]
    pub daemon: bool,

    /// Dry-run mode: fetch IP and render templates without sending live HTTP write requests
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Validate configuration syntax and interface existence without running
    #[arg(long = "check")]
    pub check: bool,

    /// List all predefined DDNS providers and their templates
    #[arg(long = "list-providers")]
    pub list_providers: bool,

    /// Query and show default request template and details for a predefined provider
    #[arg(
        long = "show-provider",
        visible_alias = "provider",
        value_name = "NAME"
    )]
    pub show_provider: Option<String>,

    /// Number of Tokio runtime worker threads (1 for single-thread lightweight runtime)
    #[arg(short = 't', long = "worker-threads", value_parser = parse_worker_threads)]
    pub worker_threads: Option<usize>,
}

fn parse_worker_threads(s: &str) -> Result<usize, String> {
    let val: usize = s
        .parse()
        .map_err(|_| format!("'{}' is not a valid number", s))?;
    if val == 0 {
        return Err("worker threads must be greater than 0".to_string());
    }
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_threads_zero_fails_cli_parsing() {
        let res = Cli::try_parse_from(["rdns", "-t", "0"]);
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(err.contains("worker threads must be greater than 0"));
    }

    #[test]
    fn test_worker_threads_positive_succeeds() {
        let cli = Cli::try_parse_from(["rdns", "-t", "4"]).unwrap();
        assert_eq!(cli.worker_threads, Some(4));
    }
}
