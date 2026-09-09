//! Assertion verification for HTTP responses.

use crate::error::RdnsError;
use regex::Regex;

pub struct ResponseVerifier {
    success_regex: Option<Regex>,
    success_contains: Vec<String>,
}

impl ResponseVerifier {
    pub fn new(
        success_regex: Option<&str>,
        success_contains: &[String],
    ) -> Result<Self, RdnsError> {
        let regex = if let Some(reg) = success_regex {
            Some(
                Regex::new(reg)
                    .map_err(|e| RdnsError::Assertion(format!("Invalid regex: {}", e)))?,
            )
        } else {
            None
        };

        Ok(Self {
            success_regex: regex,
            success_contains: success_contains.to_vec(),
        })
    }

    pub fn verify(&self, status: reqwest::StatusCode, body: &str) -> Result<(), RdnsError> {
        if !status.is_success() {
            return Err(RdnsError::Assertion(format!(
                "HTTP status was not successful: {}",
                status
            )));
        }

        if let Some(ref reg) = self.success_regex
            && !reg.is_match(body)
        {
            return Err(RdnsError::Assertion(format!(
                "Response body failed regex assertion '{}'. Body snippet: {:.200}",
                reg, body
            )));
        }

        for substr in &self.success_contains {
            if !body.contains(substr) {
                return Err(RdnsError::Assertion(format!(
                    "Response body does not contain required substring '{}'. Body snippet: {:.200}",
                    substr, body
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_status_and_regex() {
        let verifier = ResponseVerifier::new(Some("^(good|nochg)"), &[]).expect("valid regex");

        assert!(
            verifier
                .verify(reqwest::StatusCode::OK, "good 1.2.3.4")
                .is_ok()
        );
        assert!(
            verifier
                .verify(reqwest::StatusCode::OK, "nochg 1.2.3.4")
                .is_ok()
        );
        assert!(verifier.verify(reqwest::StatusCode::OK, "badauth").is_err());
        assert!(
            verifier
                .verify(reqwest::StatusCode::BAD_REQUEST, "good 1.2.3.4")
                .is_err()
        );
    }

    #[test]
    fn test_verifier_contains() {
        let verifier = ResponseVerifier::new(None, &["\"status\":\"success\"".to_string()])
            .expect("valid verifier");

        assert!(
            verifier
                .verify(reqwest::StatusCode::OK, "{\"status\":\"success\"}")
                .is_ok()
        );
        assert!(
            verifier
                .verify(reqwest::StatusCode::OK, "{\"status\":\"error\"}")
                .is_err()
        );
    }
}
