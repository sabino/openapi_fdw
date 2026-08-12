use axum::http::{HeaderMap, HeaderValue, header};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const COOKIE_NAME: &str = "openapi_fdw_session";

#[derive(Clone)]
pub struct AuthState {
    admin_token: Vec<u8>,
    session_token: String,
    cookie_secure: bool,
}

impl AuthState {
    pub fn new(admin_token: String, cookie_secure: bool) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"openapi-fdw-control-session-v1\0");
        digest.update(admin_token.as_bytes());
        let session_token = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            admin_token: admin_token.into_bytes(),
            session_token,
            cookie_secure,
        }
    }

    pub fn verify_token(&self, candidate: &str) -> bool {
        constant_time_equal(&self.admin_token, candidate.as_bytes())
    }

    pub fn authorized(&self, headers: &HeaderMap) -> bool {
        if let Some(candidate) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        {
            if self.verify_token(candidate) {
                return true;
            }
        }
        cookie_value(headers, COOKIE_NAME).is_some_and(|candidate| {
            constant_time_equal(self.session_token.as_bytes(), candidate.as_bytes())
        })
    }

    pub fn session_cookie(&self) -> HeaderValue {
        let secure = if self.cookie_secure { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{COOKIE_NAME}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200{secure}",
            self.session_token
        ))
        .expect("session cookie contains only hexadecimal characters")
    }

    pub fn clear_cookie(&self) -> HeaderValue {
        let secure = if self.cookie_secure { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}"
        ))
        .expect("clear-cookie header is static")
    }
}

pub fn has_mutation_header(headers: &HeaderMap) -> bool {
    headers
        .get("x-openapi-fdw-request")
        .and_then(|value| value.to_str().ok())
        == Some("control-plane")
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(candidate, value)| (candidate == name).then_some(value))
}

fn constant_time_equal(expected: &[u8], candidate: &[u8]) -> bool {
    expected.len() == candidate.len() && bool::from(expected.ct_eq(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_and_cookie_authenticate_without_exposing_the_admin_token() {
        let auth = AuthState::new("correct-horse-battery-staple".to_string(), false);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer correct-horse-battery-staple"),
        );
        assert!(auth.authorized(&headers));
        let session_cookie = auth.session_cookie();
        let cookie = session_cookie.to_str().unwrap();
        assert!(!cookie.contains("correct-horse-battery-staple"));
    }
}
