// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Cross-platform OS shutdown signal listening.

use std::io;

pub enum ProcessSignal {
    Shutdown(String),
    #[allow(dead_code)]
    Reload,
}

pub struct SignalListener;

impl SignalListener {
    /// Asynchronously wait for any termination or reload signal from the OS.
    pub async fn wait_signal() -> Result<ProcessSignal, io::Error> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigint = signal(SignalKind::interrupt())?;
            let mut sigterm = signal(SignalKind::terminate())?;
            let mut sighup = signal(SignalKind::hangup())?;

            tokio::select! {
                _ = sigint.recv() => Ok(ProcessSignal::Shutdown("SIGINT".to_string())),
                _ = sigterm.recv() => Ok(ProcessSignal::Shutdown("SIGTERM".to_string())),
                _ = sighup.recv() => Ok(ProcessSignal::Reload),
            }
        }

        #[cfg(windows)]
        {
            use tokio::signal::windows::{
                ctrl_break, ctrl_c, ctrl_close, ctrl_logoff, ctrl_shutdown,
            };
            let mut c_c = ctrl_c()?;
            let mut c_break = ctrl_break()?;
            let mut c_close = ctrl_close()?;
            let mut c_shutdown = ctrl_shutdown()?;
            let mut c_logoff = ctrl_logoff()?;

            tokio::select! {
                _ = c_c.recv() => Ok(ProcessSignal::Shutdown("CTRL_C".to_string())),
                _ = c_break.recv() => Ok(ProcessSignal::Shutdown("CTRL_BREAK".to_string())),
                _ = c_close.recv() => Ok(ProcessSignal::Shutdown("CTRL_CLOSE".to_string())),
                _ = c_shutdown.recv() => Ok(ProcessSignal::Shutdown("CTRL_SHUTDOWN".to_string())),
                _ = c_logoff.recv() => Ok(ProcessSignal::Shutdown("CTRL_LOGOFF".to_string())),
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            tokio::signal::ctrl_c().await?;
            Ok(ProcessSignal::Shutdown("CTRL_C".to_string()))
        }
    }
}
