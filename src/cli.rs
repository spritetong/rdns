// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Command-line interface definitions and parsing.

use clap::Parser;
use std::path::PathBuf;

/// RDNS - Modern, Lightweight, Webhook-driven DDNS Client
#[derive(Parser, Debug, Clone)]
#[command(
    name = "rdns",
    version,
    about,
    long_about = None,
    after_help = "Cloudflare Helper:\n  Use 'python scripts/cf_lookup.py -d <DOMAIN>' to query Zone ID and Record IDs from Cloudflare."
)]
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

    /// Query and show default request template and details for a predefined provider (for Cloudflare, see also 'python scripts/cf_lookup.py')
    #[arg(
        long = "show-provider",
        visible_alias = "provider",
        value_name = "NAME"
    )]
    pub show_provider: Option<String>,

    /// Number of Tokio runtime worker threads (1 for single-thread lightweight runtime)
    #[arg(short = 't', long = "worker-threads", value_parser = parse_worker_threads)]
    pub worker_threads: Option<usize>,

    /// Override the path to the state persistence JSON file
    #[arg(short = 's', long = "state", value_name = "PATH")]
    pub state: Option<PathBuf>,

    /// Control whether to write the state persistence file to disk (true/false)
    #[arg(
        long = "write-state",
        visible_alias = "save-state",
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new(),
        action = clap::ArgAction::Set
    )]
    pub write_state: Option<bool>,

    /// Disable writing the state persistence file to disk
    #[arg(
        long = "no-state",
        visible_aliases = ["no-state-file", "no-save-state"]
    )]
    pub no_state: bool,

    /// Disable netwatcher event-driven network change detection (fallback to timer polling)
    #[arg(long = "no-netwatcher")]
    pub no_netwatcher: bool,

    /// Log level filter (trace, debug, info, warn, error, off) [default: info]
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        value_enum,
        ignore_case = true
    )]
    pub log_level: Option<LogLevel>,
}

/// Supported logging levels for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
            LogLevel::Off => "off",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Cli {
    /// Determines whether state file persistence to disk should be performed.
    pub fn should_write_state(&self) -> bool {
        if self.no_state {
            return false;
        }
        self.write_state.unwrap_or(true)
    }

    /// Returns the configured log level or defaults to `LogLevel::Info`.
    #[allow(dead_code)]
    pub fn log_level(&self) -> LogLevel {
        self.log_level.unwrap_or(LogLevel::Info)
    }
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

    #[test]
    fn test_state_arg_parsed() {
        let cli = Cli::try_parse_from(["rdns", "--state", "/tmp/custom_state.json"]).unwrap();
        assert_eq!(cli.state, Some(PathBuf::from("/tmp/custom_state.json")));
        assert!(cli.should_write_state());
    }

    #[test]
    fn test_default_should_write_state_true() {
        let cli = Cli::try_parse_from(["rdns"]).unwrap();
        assert!(cli.should_write_state());
    }

    #[test]
    fn test_no_state_flag_disables_writing() {
        let cli = Cli::try_parse_from(["rdns", "--no-state"]).unwrap();
        assert!(!cli.should_write_state());

        let cli_alias = Cli::try_parse_from(["rdns", "--no-state-file"]).unwrap();
        assert!(!cli_alias.should_write_state());

        let cli_alias2 = Cli::try_parse_from(["rdns", "--no-save-state"]).unwrap();
        assert!(!cli_alias2.should_write_state());
    }

    #[test]
    fn test_write_state_option_parsing() {
        let cli_false = Cli::try_parse_from(["rdns", "--write-state", "false"]).unwrap();
        assert!(!cli_false.should_write_state());

        let cli_zero = Cli::try_parse_from(["rdns", "--write-state", "0"]).unwrap();
        assert!(!cli_zero.should_write_state());

        let cli_true = Cli::try_parse_from(["rdns", "--write-state", "true"]).unwrap();
        assert!(cli_true.should_write_state());

        let cli_flag = Cli::try_parse_from(["rdns", "--write-state"]).unwrap();
        assert!(cli_flag.should_write_state());

        let cli_save = Cli::try_parse_from(["rdns", "--save-state", "false"]).unwrap();
        assert!(!cli_save.should_write_state());
    }

    #[test]
    fn test_default_log_level_is_info() {
        let cli = Cli::try_parse_from(["rdns"]).unwrap();
        assert_eq!(cli.log_level, None);
        assert_eq!(cli.log_level(), LogLevel::Info);
    }

    #[test]
    fn test_log_level_off_parsed() {
        let cli = Cli::try_parse_from(["rdns", "--log-level", "off"]).unwrap();
        assert_eq!(cli.log_level, Some(LogLevel::Off));
        assert_eq!(cli.log_level(), LogLevel::Off);

        let cli_short = Cli::try_parse_from(["rdns", "-l", "off"]).unwrap();
        assert_eq!(cli_short.log_level, Some(LogLevel::Off));
    }

    #[test]
    fn test_log_level_all_variants_case_insensitive() {
        for (arg, expected) in [
            ("trace", LogLevel::Trace),
            ("DEBUG", LogLevel::Debug),
            ("Info", LogLevel::Info),
            ("WARN", LogLevel::Warn),
            ("error", LogLevel::Error),
            ("OFF", LogLevel::Off),
        ] {
            let cli = Cli::try_parse_from(["rdns", "-l", arg]).unwrap();
            assert_eq!(cli.log_level, Some(expected));
            assert_eq!(cli.log_level().as_str(), expected.as_str());
        }
    }

    #[test]
    fn test_log_level_invalid_value_fails() {
        let res = Cli::try_parse_from(["rdns", "-l", "verbose"]);
        assert!(res.is_err());
    }
}
