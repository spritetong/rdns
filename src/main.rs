//! RDNS - Modern, Lightweight, Webhook-driven DDNS Client.

mod cli;
mod config;
mod engine;
mod error;
mod ip;
mod lifecycle;
mod notification;
mod persistence;
pub mod provider;
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

    // 0. Provider inspection commands (can run without config file)
    if cli.list_providers {
        print!("{}", provider::format_providers_list());
        return ExitCode::SUCCESS;
    }

    if let Some(ref provider_name) = cli.show_provider {
        match provider::format_provider_detail(provider_name) {
            Ok(detail) => {
                print!("{}", detail);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    // 1. Check mode
    if cli.check {
        match load_config(&cli.config) {
            Ok(mut cfg) => match validate_config(&mut cfg) {
                Ok(()) => {
                    let host_interfaces = match ifaddrsx::get_interfaces(false) {
                        Ok(ifaces) => ifaces,
                        Err(e) => {
                            eprintln!("Failed to enumerate host network interfaces: {}", e);
                            return ExitCode::FAILURE;
                        }
                    };

                    let mut all_matched = true;
                    for iface in &cfg.interfaces {
                        if let Some(ref v4) = iface.ipv4
                            && v4.enabled
                            && v4.source == "interface"
                        {
                            let pattern = v4.interface.as_deref().unwrap_or(".*");
                            if let Ok(reg) = regex::Regex::new(pattern) {
                                let matched = host_interfaces.iter().any(|hi| {
                                    reg.is_match(&hi.name) || reg.is_match(hi.friendly_name())
                                });
                                if !matched {
                                    eprintln!(
                                        "Interface validation failed: interface '{}' IPv4 pattern '{}' matched no host network interfaces",
                                        iface.name, pattern
                                    );
                                    all_matched = false;
                                }
                            }
                        }
                        if let Some(ref v6) = iface.ipv6
                            && v6.enabled
                            && v6.source == "interface"
                        {
                            let pattern = v6.interface.as_deref().unwrap_or(".*");
                            if let Ok(reg) = regex::Regex::new(pattern) {
                                let matched = host_interfaces.iter().any(|hi| {
                                    reg.is_match(&hi.name) || reg.is_match(hi.friendly_name())
                                });
                                if !matched {
                                    eprintln!(
                                        "Interface validation failed: interface '{}' IPv6 pattern '{}' matched no host network interfaces",
                                        iface.name, pattern
                                    );
                                    all_matched = false;
                                }
                            }
                        }
                    }

                    if !all_matched {
                        return ExitCode::FAILURE;
                    }

                    println!(
                        "Configuration '{}' is valid. Interfaces: {}, Tasks: {}",
                        cli.config.display(),
                        cfg.interfaces.len(),
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
        Ok(mut cfg) => match validate_config(&mut cfg) {
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
        if threads == 0 {
            eprintln!("Worker threads must be greater than 0");
            return ExitCode::FAILURE;
        }
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
        let state_path = cli
            .state
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("state.json"));
        let state_store = StateStore::new(&state_path);

        if cli.dry_run || cli.once {
            let lifecycle = Arc::new(LifecycleManager::new(config.global.shutdown_timeout));
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

            tracing::info!(dry_run = cli.dry_run, "Executing single run cycle");
            match scheduler.run_once().await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!(error = %e, "Single run completed with errors");
                    ExitCode::FAILURE
                }
            }
        } else {
            // Default or --daemon: long-running service with SIGHUP reload support
            let mut current_config = config;
            loop {
                let lifecycle = Arc::new(LifecycleManager::new(current_config.global.shutdown_timeout));
                let scheduler = match SchedulerService::new(
                    &current_config,
                    state_store.clone(),
                    Arc::clone(&lifecycle),
                    cli.dry_run,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to initialize scheduler service");
                        return ExitCode::FAILURE;
                    }
                };

                let action = scheduler.run_daemon().await;
                match action {
                    lifecycle::LifecycleAction::Shutdown => {
                        return ExitCode::SUCCESS;
                    }
                    lifecycle::LifecycleAction::Reload => {
                        tracing::info!(
                            path = %cli.config.display(),
                            "Received SIGHUP signal; reloading configuration"
                        );
                        match load_config(&cli.config) {
                            Ok(mut new_cfg) => match validate_config(&mut new_cfg) {
                                Ok(()) => {
                                    tracing::info!("Configuration reloaded and validated successfully");
                                    current_config = new_cfg;
                                }
                                Err(e) => {
                                    tracing::error!(
                                        error = %e,
                                        "Reloaded configuration failed validation; keeping current configuration"
                                    );
                                }
                            },
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "Failed to read reloaded configuration; keeping current configuration"
                                );
                            }
                        }
                    }
                }
            }
        }
    })
}
