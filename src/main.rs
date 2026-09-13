// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

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
pub mod watcher;

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
    let mut cli = Cli::parse();

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
                        let eff_v4 = iface.effective_ipv4_strategy();
                        if let Some(ref v4) = eff_v4
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
                        let eff_v6 = iface.effective_ipv6_strategy();
                        if let Some(ref v6) = eff_v6
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
    let mut config = match load_config(&cli.config) {
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

    // Override `no_state` flag
    if !cli.no_state
        && let Some(no_state) = config.global.no_state
    {
        cli.no_state = no_state;
    }

    // Override `netwatcher` flag
    if cli.no_netwatcher {
        config.global.netwatcher = false;
    }

    // 3. Initialize logging
    let filter = if let Some(lvl) = cli.log_level {
        EnvFilter::new(lvl.as_str())
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&config.global.log_level))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    tracing::info!("Starting RDNS client v{}", env!("CARGO_PKG_VERSION"));

    // 4. Determine Tokio worker threads
    let worker_threads = cli
        .worker_threads
        .or(config.global.worker_threads)
        .map(|t| t as u32);

    let runtime = match build_tokio_runtime(worker_threads) {
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
        let write_state = cli.should_write_state();
        if !write_state {
            tracing::info!(
                "State persistence disk writing is disabled ('{}')",
                state_path.display()
            );
        }
        let state_store = StateStore::new_with_write_flag(&state_path, write_state);

        if cli.dry_run || cli.once {
            let lifecycle = Arc::new(LifecycleManager::new(config.global.shutdown_timeout));
            let scheduler = match SchedulerService::new(
                &config,
                state_store.clone(),
                Arc::clone(&lifecycle),
                cli.dry_run,
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to initialize scheduler service: {}", e);
                    return ExitCode::FAILURE;
                }
            };

            tracing::info!("Executing single run cycle");
            let result = scheduler.run_once().await;
            // Flush state persistence before process exit; the background writer is
            // dropped when the runtime shuts down and would lose pending updates.
            state_store.flush().await;
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!("Single run completed with errors: {}", e);
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
                        tracing::error!("Failed to initialize scheduler service: {}", e);
                        return ExitCode::FAILURE;
                    }
                };

                let action = scheduler.run_daemon().await;
                // Guarantee trailing state updates are persisted before exit or reload.
                state_store.flush().await;
                match action {
                    lifecycle::LifecycleAction::Shutdown => {
                        return ExitCode::SUCCESS;
                    }
                    lifecycle::LifecycleAction::Reload => {
                        tracing::info!(
                            "Received SIGHUP signal, reloading configuration from '{}'",
                            cli.config.display()
                        );
                        match load_config(&cli.config) {
                            Ok(mut new_cfg) => match validate_config(&mut new_cfg) {
                                Ok(()) => {
                                    tracing::info!("Configuration reloaded and validated successfully");
                                    current_config = new_cfg;
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Reloaded configuration failed validation: {}",
                                        e
                                    );
                                }
                            },
                            Err(e) => {
                                tracing::error!(
                                    "Failed to read reloaded configuration: {}",
                                    e
                                );
                            }
                        }
                    }
                }
            }
        }
    })
}

pub(crate) fn build_tokio_runtime(
    nb_worker_threads: Option<u32>,
) -> std::io::Result<tokio::runtime::Runtime> {
    let threads = nb_worker_threads.or_else(|| {
        std::env::var("TOKIO_WORKER_THREADS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
    });

    let mut builder = match threads {
        Some(1) => {
            tracing::info!("Initialized single-threaded (current_thread) Tokio runtime");
            tokio::runtime::Builder::new_current_thread()
        }
        Some(n) => {
            tracing::info!(
                "Initialized multi-threaded Tokio runtime with {} worker threads",
                n
            );
            let mut b = tokio::runtime::Builder::new_multi_thread();
            b.worker_threads(n as usize);
            b
        }
        None => tokio::runtime::Builder::new_multi_thread(),
    };

    builder.enable_all().build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_tokio_runtime_single_thread() {
        let rt = build_tokio_runtime(Some(1)).expect("must build current_thread runtime");
        let res = rt.block_on(async { tokio::spawn(async { 42 }).await.unwrap() });
        assert_eq!(res, 42);
    }

    #[test]
    fn test_build_tokio_runtime_multi_thread() {
        let rt = build_tokio_runtime(Some(2)).expect("must build multi_thread runtime");
        let res = rt.block_on(async { tokio::spawn(async { 100 }).await.unwrap() });
        assert_eq!(res, 100);
    }

    #[test]
    fn test_build_tokio_runtime_default() {
        let rt = build_tokio_runtime(None).expect("must build default runtime");
        let res = rt.block_on(async { tokio::spawn(async { 200 }).await.unwrap() });
        assert_eq!(res, 200);
    }
}
