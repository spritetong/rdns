//! Cross-platform OS shutdown signal listening.

use std::io;

pub struct SignalListener;

impl SignalListener {
    /// Asynchronously wait for any termination or interrupt signal from the OS.
    pub async fn wait_shutdown_signal() -> Result<String, io::Error> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigint = signal(SignalKind::interrupt())?;
            let mut sigterm = signal(SignalKind::terminate())?;

            tokio::select! {
                _ = sigint.recv() => Ok("SIGINT".to_string()),
                _ = sigterm.recv() => Ok("SIGTERM".to_string()),
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
                _ = c_c.recv() => Ok("CTRL_C".to_string()),
                _ = c_break.recv() => Ok("CTRL_BREAK".to_string()),
                _ = c_close.recv() => Ok("CTRL_CLOSE".to_string()),
                _ = c_shutdown.recv() => Ok("CTRL_SHUTDOWN".to_string()),
                _ = c_logoff.recv() => Ok("CTRL_LOGOFF".to_string()),
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            tokio::signal::ctrl_c().await?;
            Ok("CTRL_C".to_string())
        }
    }
}
