//! HTTP Webhook Engine facade.

mod client;
mod executor;
mod template;
mod verifier;

pub use client::build_http_client;
pub use executor::RequestExecutor;
pub use template::TemplateContext;

use crate::config::GlobalConfig;
use crate::error::RdnsError;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct HttpEngine {
    executor: Arc<RequestExecutor>,
}

impl HttpEngine {
    pub fn new(global: &GlobalConfig) -> Result<Self, RdnsError> {
        let timeout = Duration::from_secs(global.timeout);
        let client = build_http_client(
            timeout,
            global.proxy.as_deref(),
            false,
            global.cacerts.as_deref(),
        )?;
        Ok(Self {
            executor: Arc::new(RequestExecutor::new(
                client,
                global.proxy.clone(),
                global.cacerts.clone(),
                timeout,
            )),
        })
    }

    pub fn executor(&self) -> &RequestExecutor {
        &self.executor
    }
}
