//! RDNS - Modern, Lightweight, Webhook-driven DDNS Client.

mod cli;
mod config;
mod engine;
mod error;
mod ip;
mod lifecycle;
mod notification;
mod persistence;
mod scheduler;

use clap::Parser;
use cli::Cli;
use config::{load_config, validate_config};
use lifecycle::LifecycleManager;
use persistence::StateStore;
use scheduler::SchedulerService;
use std::process::ExitCode;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    let cli = Cli::parse();

    // 1. Check mode
    if cli.check {
        match load_config(&cli.config) {
            Ok(cfg) => match validate_config(&cfg) {
                Ok(()) => {
                    println!(
                        "Configuration '{}' is valid. Tasks configured: {}",
                        cli.config.display(),
                        cfg.tasks.len()
                    );
                    return ExitCode::SUCCESS;
                }
                Err(e) => {
                    eprintln!("Configuration validation failed: {}", e);
                    return ExitCode::FAILURE;
                }
            },
            Err(e) => {
                eprintln!("Failed to load configuration: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    // 2. Load and validate config
    let config = match load_config(&cli.config) {
        Ok(cfg) => match validate_config(&cfg) {
            Ok(()) => cfg,
            Err(e) => {
                eprintln!("Configuration validation failed: {}", e);
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            eprintln!(
                "Failed to load configuration from '{}': {}",
                cli.config.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    // 3. Initialize logging
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.global.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Starting RDNS client");

    // 4. Determine Tokio worker threads
    let worker_threads = cli.worker_threads.or(config.global.worker_threads);

    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if let Some(threads) = worker_threads {
        tracing::info!(
            worker_threads = threads,
            "Configuring custom worker threads count"
        );
        builder.worker_threads(threads);
    }

    let runtime = match builder.build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to build Tokio runtime: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // 5. Execute within Tokio runtime
    runtime.block_on(async {
        let lifecycle = Arc::new(LifecycleManager::new(config.global.shutdown_timeout));
        let state_store = StateStore::new("state.json");

        let scheduler = match SchedulerService::new(
            &config,
            state_store,
            Arc::clone(&lifecycle),
            cli.dry_run,
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize scheduler service");
                return ExitCode::FAILURE;
            }
        };

        if cli.dry_run || cli.once {
            tracing::info!(dry_run = cli.dry_run, "Executing single run cycle");
            match scheduler.run_once().await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!(error = %e, "Single run completed with errors");
                    ExitCode::FAILURE
                }
            }
        } else {
            // Default or --daemon: long-running service
            scheduler.run_daemon().await;
            ExitCode::SUCCESS
        }
    })
}
