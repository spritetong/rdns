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

    /// Number of Tokio runtime worker threads (1 for single-thread lightweight runtime)
    #[arg(short = 't', long = "worker-threads")]
    pub worker_threads: Option<usize>,
}
