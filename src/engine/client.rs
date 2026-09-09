//! HTTP Client construction with rustls and platform native roots.

use std::time::Duration;

pub fn build_http_client(
    timeout: Duration,
    proxy: Option<&str>,
    tls_insecure: bool,
) -> Result<reqwest::Client, reqwest::Error> {
    let mut builder = reqwest::Client::builder().timeout(timeout).use_rustls_tls();

    if tls_insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(proxy_url) = proxy
        && !proxy_url.trim().is_empty()
    {
        let p = reqwest::Proxy::all(proxy_url)?;
        builder = builder.proxy(p);
    }

    builder.build()
}
