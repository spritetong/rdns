// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! HTTP Client construction with rustls, manual platform native roots, and custom CA certs.

use crate::error::{ConfigError, RdnsError};
use std::time::Duration;

/// Load custom CA certificates from a file on disk.
///
/// Supports PEM bundles (multiple concatenated certificates), single PEM certificates,
/// or binary DER certificates.
pub fn load_custom_ca_certs(path: &str) -> Result<Vec<reqwest::tls::Certificate>, RdnsError> {
    let content = std::fs::read(path).map_err(|e| {
        RdnsError::Config(ConfigError::ReadFile {
            path: path.to_string(),
            source: e,
        })
    })?;

    // 1. Try parsing as PEM (handles both single certificate and multi-certificate bundles)
    if let Ok(certs) = reqwest::tls::Certificate::from_pem_bundle(&content)
        && !certs.is_empty()
    {
        return Ok(certs);
    }

    // 2. Try DER format (must start with ASN.1 SEQUENCE 0x30 and have plausible cert length >= 64)
    if content.starts_with(&[0x30])
        && content.len() >= 64
        && let Ok(cert) = reqwest::tls::Certificate::from_der(&content)
    {
        return Ok(vec![cert]);
    }

    Err(RdnsError::Config(ConfigError::Validation {
        field: "cacerts".to_string(),
        message: format!(
            "Failed to parse valid CA certificates from '{}' (expected valid PEM or DER format)",
            path
        ),
    }))
}

/// Construct a `reqwest::Client` with rustls, manually loaded native root certificates,
/// optional custom CA certificates, proxy configuration, and timeout.
pub fn build_http_client(
    timeout: Duration,
    proxy: Option<&str>,
    tls_insecure: bool,
    cacerts_path: Option<&str>,
) -> Result<reqwest::Client, RdnsError> {
    let mut builder = reqwest::Client::builder().timeout(timeout).use_rustls_tls();

    // 1. Manually load OS platform native CA root certificates via rustls-native-certs
    let native_res = rustls_native_certs::load_native_certs();
    for cert in native_res.certs {
        if let Ok(c) = reqwest::tls::Certificate::from_der(cert.as_ref()) {
            builder = builder.add_root_certificate(c);
        }
    }
    for err in native_res.errors {
        tracing::debug!("Failed to load a native root certificate: {}", err);
    }

    // 2. Load custom CA certificates file if configured
    if let Some(path) = cacerts_path
        && !path.trim().is_empty()
    {
        let custom_certs = load_custom_ca_certs(path)?;
        tracing::info!(
            "Loaded {} custom CA certificate(s) from '{}'",
            custom_certs.len(),
            path
        );
        for cert in custom_certs {
            builder = builder.add_root_certificate(cert);
        }
    }

    if tls_insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(proxy_url) = proxy
        && !proxy_url.trim().is_empty()
    {
        let p = reqwest::Proxy::all(proxy_url).map_err(|e| {
            RdnsError::Config(ConfigError::Validation {
                field: "proxy".to_string(),
                message: format!("Invalid proxy URL '{}': {}", proxy_url, e),
            })
        })?;
        builder = builder.proxy(p);
    }

    builder.build().map_err(RdnsError::Http)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TEST_PEM: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw\n\
TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh\n\
cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4\n\
WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu\n\
ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY\n\
MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc\n\
h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+\n\
0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK/62gvQUJUe\n\
A4NIBPB40V5rfNLuMWJKascoeqqmIembuoir8STjPa37i4y42KwMWNU7GQV3Abb6\n\
s/mUz6qvMtVDGaqLHex8/5a65yhZG6U9enElhxxXQ8J6TrHMUEYJKvpaLaQXQXNT\n\
kxNKN906gzZZhrJZBgWC1gg+phk45U197QohurRmZ3PNBpwdL036t/B5jdEkXVqq\n\
tkemAXTumVNnZny600eqZycDz4IkRj/WG09rsBCpRMWY1RqK2HMMNCnDYCmIOpdL\n\
OmGJ66WcPzPziRSCYG25GEKAFlGasMgsp21hW3QsVeAxzs5BrDAqNfaSTvcVcRgq\n\
U9lVCuhlPpVVR6CYqFoLSZ5PPughdVqhHptezNxhom8Tr6BlDLnow2BzlkSuW3TU\n\
fa6l/WwGhGiDWXdCmKsNXrhPFGQrakUISVDyNiMGFiLuy279K+6D3qVepeGeyVnE\n\
wNxYBzvEQTWgz3h97u42AwpLXXTT5QSifaFVZoPF5s1Lt6Yo3AiPjnu42cw63yyS\n\
YUYRMW63qQ0GDNxf9NdgovDLAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV\n\
HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgBltwAj43ev6jDANBgkq\n\
hkiG9w0BAQsFAAOCAgEAVR9YqbyhurdtNxSfManRqzTx50Srtq26/hITn40NN90P\n\
jdn8ssmVC35x496gg5LXMggyt98UOiIXWXu0NzkeHE4QGvnR4YcqIeh8qnQKFs54\n\
FQNPENOi6V9WOMPf/VNn4WHqx8zsHSTXC2MMKaLgE0q9THFluEZl8MiEBwys56Gi\n\
40pdHM5hucR2UFQ744A8XXcxS6AcxAE1UevbQEcFYGtTym0up2A7lPBQ4q42TRxH\n\
/NJ5Mtx+QwV+DLINhqv6zzZ//35jJNaKIuoPxKAWHS5jXlSr63LTr1ITxSg/VnWD\n\
noOAWA37bkwRwufGaE+FvqxoduEB5o73XnYO8ScdgBp475SX461ao8audeJ8025U\n\
wSrv776ZcC5V9G75tJ/66LnRWyC92NIfzKH5jbmkg5762XAUeeP59dZwvDL32qqP\n\
wL4m4QUK13/LxhUMAFqkWIoTMuumfKGneRejbravUhMgVeggkPSVE3/OpyVaABU9\n\
OycnVCXECKGETut+QC/omL7828nfbihCghnqZaXPjMr5cSuE18CGXWkVRrxg307m\n\
PpOXlNoA52r0Yhk3G/H2JWBq12lYO1/wU45aS1wC48qcp1bkW6SjVCUkCWuhIKGo\n\
dBmEW53UCN3U5NzyrmWGmEtqOLGDbxXXdK3VNGN51GQTu75htA9l7265UV5nZME=\n\
-----END CERTIFICATE-----\n";

    fn write_temp_cert(content: &[u8]) -> (std::path::PathBuf, impl Drop) {
        let path = std::env::temp_dir().join(format!(
            "rdns_test_cert_{}.pem",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, content).unwrap();
        struct Cleaner(std::path::PathBuf);
        impl Drop for Cleaner {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let cleaner = Cleaner(path.clone());
        (path, cleaner)
    }

    #[test]
    fn test_load_custom_ca_certs_valid_pem() {
        let (path, _cleaner) = write_temp_cert(VALID_TEST_PEM.as_bytes());
        let certs = load_custom_ca_certs(path.to_str().unwrap()).unwrap();
        assert_eq!(certs.len(), 1);
    }

    #[test]
    fn test_load_custom_ca_certs_missing_file() {
        let res = load_custom_ca_certs("non_existent_file_ca.pem");
        assert!(res.is_err());
    }

    #[test]
    fn test_load_custom_ca_certs_invalid_content() {
        let (path, _cleaner) = write_temp_cert(b"not a valid certificate content");
        let res = load_custom_ca_certs(path.to_str().unwrap());
        assert!(res.is_err());
    }

    #[test]
    fn test_build_http_client_with_native_roots() {
        let client = build_http_client(Duration::from_secs(5), None, false, None);
        assert!(client.is_ok());
    }

    #[test]
    fn test_build_http_client_with_custom_certs() {
        let (path, _cleaner) = write_temp_cert(VALID_TEST_PEM.as_bytes());
        let client = build_http_client(
            Duration::from_secs(5),
            None,
            false,
            Some(path.to_str().unwrap()),
        );
        assert!(client.is_ok());
    }
}
