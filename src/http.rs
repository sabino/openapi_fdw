use crate::error::{OpenApiFdwError, Result};
use crate::options::{Auth, ServerConfig};
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, LINK, RETRY_AFTER};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};
use url::Url;

#[derive(Clone, Debug)]
pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) url: Url,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) body: Option<Value>,
}

#[derive(Debug)]
pub(crate) struct HttpResponse {
    pub(crate) value: Value,
    pub(crate) link_headers: Vec<String>,
    pub(crate) effective_url: Url,
}

#[derive(Clone, Copy, Debug, Eq)]
struct ClientKey {
    connect_timeout_ms: u64,
    max_redirects: usize,
}

impl PartialEq for ClientKey {
    fn eq(&self, other: &Self) -> bool {
        self.connect_timeout_ms == other.connect_timeout_ms
            && self.max_redirects == other.max_redirects
    }
}

impl Hash for ClientKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.connect_timeout_ms.hash(state);
        self.max_redirects.hash(state);
    }
}

static CLIENTS: OnceLock<Mutex<HashMap<ClientKey, Client>>> = OnceLock::new();

pub(crate) fn endpoint_url(base: &Url, endpoint: &str) -> Result<Url> {
    let raw = format!(
        "{}/{}",
        base.as_str().trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    );
    Url::parse(&raw).map_err(|_| {
        OpenApiFdwError::Configuration(
            "the server base_url and table endpoint do not form a valid URL".to_string(),
        )
    })
}

pub(crate) fn resolve_page_url(
    base: &Url,
    current: &Url,
    raw: &str,
    allow_cross_origin: bool,
) -> Result<Url> {
    let next = if raw.starts_with("http://") || raw.starts_with("https://") {
        Url::parse(raw)
    } else if let Some(query) = raw.strip_prefix('?') {
        let mut url = current.clone();
        url.set_query(Some(query));
        Ok(url)
    } else if raw.starts_with('/') {
        base.join(raw)
    } else {
        current.join(raw)
    }
    .map_err(|_| OpenApiFdwError::Response("API returned an invalid next-page URL".to_string()))?;

    if !matches!(next.scheme(), "http" | "https") {
        return Err(OpenApiFdwError::Response(
            "API returned a next-page URL with an unsupported scheme".to_string(),
        ));
    }
    if !allow_cross_origin && !same_origin(base, &next) {
        return Err(OpenApiFdwError::Response(format!(
            "API returned a cross-origin next-page URL for {}; refusing to forward credentials",
            display_origin(&next)
        )));
    }
    Ok(next)
}

pub(crate) fn execute_json(server: &ServerConfig, request: &HttpRequest) -> Result<HttpResponse> {
    let raw = execute(server, request)?;
    if let Some(content_type) = raw.content_type.as_deref()
        && !is_json_content_type(content_type)
    {
        return Err(OpenApiFdwError::Response(format!(
            "response Content-Type `{content_type}` is not JSON"
        )));
    }
    let value = serde_json::from_slice(&raw.body).map_err(|error| {
        OpenApiFdwError::Response(format!(
            "response body is not valid JSON at line {}, column {}",
            error.line(),
            error.column()
        ))
    })?;
    Ok(HttpResponse {
        value,
        link_headers: raw.link_headers,
        effective_url: raw.effective_url,
    })
}

pub(crate) fn execute_mutation(server: &ServerConfig, request: &HttpRequest) -> Result<()> {
    execute(server, request).map(|_| ())
}

pub(crate) fn fetch_spec(server: &ServerConfig) -> Result<Value> {
    if let Some(raw) = &server.spec_json {
        return parse_spec_document(raw.as_bytes());
    }
    let url = server.spec_url.clone().ok_or_else(|| {
        OpenApiFdwError::Spec(
            "schema import requires server option `spec_url` or `spec_json`".to_string(),
        )
    })?;
    let spec_server = server.for_spec_fetch();
    let response = execute(
        &spec_server,
        &HttpRequest {
            method: "GET".to_string(),
            url,
            query: Vec::new(),
            body: None,
        },
    )?;
    parse_spec_document(&response.body)
}

fn parse_spec_document(raw: &[u8]) -> Result<Value> {
    serde_json::from_slice(raw)
        .or_else(|_| serde_yaml_ng::from_slice(raw))
        .map_err(|error| {
            OpenApiFdwError::Spec(format!(
                "document is neither valid JSON nor valid YAML: {error}"
            ))
        })
}

struct RawResponse {
    body: Vec<u8>,
    content_type: Option<String>,
    link_headers: Vec<String>,
    effective_url: Url,
}

fn execute(server: &ServerConfig, request: &HttpRequest) -> Result<RawResponse> {
    let client = pooled_client(server)?;
    let method = Method::from_bytes(request.method.as_bytes()).map_err(|_| {
        OpenApiFdwError::Configuration("table method is not a valid HTTP method".to_string())
    })?;
    let headers = request_headers(server, request.body.is_some())?;

    // Retrying POST or PATCH after losing the response can apply the same
    // mutation twice. Only methods whose HTTP semantics are idempotent receive
    // automatic transport/status retries. An API-specific idempotency key can
    // still be supplied as a configured header, but we never assume one.
    let maximum_retries = if automatically_retryable(&method) {
        server.max_retries
    } else {
        0
    };
    let mut last_error = None;
    for attempt in 0..=maximum_retries {
        let mut url = request.url.clone();
        {
            let mut query = url.query_pairs_mut();
            for (name, value) in &request.query {
                query.append_pair(name, value);
            }
            if let Auth::Query { name, value } = &server.auth {
                query.append_pair(name, value);
            }
        }

        let mut builder = client
            .request(method.clone(), url)
            .headers(headers.clone())
            .timeout(server.request_timeout);
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }

        match builder.send() {
            Ok(response) if response.status().is_success() => {
                return consume_success(response, server.max_response_bytes);
            }
            Ok(response) => {
                let status = response.status();
                let retry_delay = retry_delay(&response, attempt, server.max_retry_delay);
                if is_retryable_status(status) && attempt < maximum_retries {
                    drop(response);
                    thread::sleep(retry_delay);
                    continue;
                }
                let excerpt = read_error_excerpt(response, server.max_response_bytes.min(4096));
                let suffix = excerpt
                    .filter(|body| !body.trim().is_empty())
                    .map(|body| format!(": {}", server.redact(body.trim())))
                    .unwrap_or_default();
                return Err(OpenApiFdwError::Http(format!(
                    "remote service returned HTTP {status}{suffix}"
                )));
            }
            Err(error) => {
                let retryable = error.is_timeout() || error.is_connect() || error.is_request();
                last_error = Some(server.redact(&error.to_string()));
                if retryable && attempt < maximum_retries {
                    thread::sleep(exponential_delay(attempt, server.max_retry_delay));
                    continue;
                }
                break;
            }
        }
    }

    Err(OpenApiFdwError::Http(last_error.unwrap_or_else(|| {
        "request failed without an error message".to_string()
    })))
}

fn automatically_retryable(method: &Method) -> bool {
    method == Method::GET
        || method == Method::HEAD
        || method == Method::PUT
        || method == Method::DELETE
        || method == Method::OPTIONS
        || method == Method::TRACE
}

fn pooled_client(server: &ServerConfig) -> Result<Client> {
    crate::initialize_tls();
    let key = ClientKey {
        connect_timeout_ms: server.connect_timeout.as_millis() as u64,
        max_redirects: server.max_redirects,
    };
    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut clients = clients
        .lock()
        .map_err(|_| OpenApiFdwError::Http("HTTP connection pool lock was poisoned".to_string()))?;
    if let Some(client) = clients.get(&key) {
        return Ok(client.clone());
    }

    let redirects = if server.max_redirects == 0 {
        reqwest::redirect::Policy::none()
    } else {
        let maximum = server.max_redirects;
        reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= maximum {
                return attempt.error("too many redirects");
            }
            if let Some(original) = attempt.previous().first()
                && !same_origin(original, attempt.url())
            {
                return attempt.error("cross-origin redirects are disabled");
            }
            attempt.follow()
        })
    };
    let client = Client::builder()
        .connect_timeout(server.connect_timeout)
        .redirect(redirects)
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .build()
        .map_err(|error| {
            OpenApiFdwError::Http(format!("could not initialize HTTP client: {error}"))
        })?;
    clients.insert(key, client.clone());
    Ok(client)
}

fn request_headers(server: &ServerConfig, has_body: bool) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in &server.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            OpenApiFdwError::Configuration(format!("`{name}` is not a valid HTTP header name"))
        })?;
        let value = HeaderValue::from_str(value).map_err(|_| {
            OpenApiFdwError::Configuration(
                "an HTTP header contains characters that cannot be transmitted".to_string(),
            )
        })?;
        headers.insert(name, value);
    }
    if let Auth::Header { name, value } = &server.auth {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            OpenApiFdwError::Configuration("api_key_name is not a valid header name".to_string())
        })?;
        let value = HeaderValue::from_str(value).map_err(|_| {
            OpenApiFdwError::Configuration(
                "the configured credential cannot be represented as an HTTP header".to_string(),
            )
        })?;
        headers.insert(name, value);
    }
    if has_body && !headers.contains_key(reqwest::header::CONTENT_TYPE) {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }
    Ok(headers)
}

fn consume_success(mut response: Response, maximum: usize) -> Result<RawResponse> {
    if let Some(length) = response.content_length()
        && length > maximum as u64
    {
        return Err(OpenApiFdwError::Response(format!(
            "response Content-Length is {length} bytes, above max_response_bytes {maximum}"
        )));
    }
    let effective_url = response.url().clone();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let link_headers = response
        .headers()
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(RawResponse {
            body: b"null".to_vec(),
            content_type,
            link_headers,
            effective_url,
        });
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(maximum as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|error| {
            OpenApiFdwError::Response(format!("could not read response body: {error}"))
        })?;
    if body.len() > maximum {
        return Err(OpenApiFdwError::Response(format!(
            "response exceeded max_response_bytes {maximum}"
        )));
    }
    Ok(RawResponse {
        body,
        content_type,
        link_headers,
        effective_url,
    })
}

fn is_json_content_type(raw: &str) -> bool {
    let media_type = raw
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type == "application/json" || media_type.ends_with("+json")
}

fn read_error_excerpt(mut response: Response, maximum: usize) -> Option<String> {
    let mut body = Vec::new();
    response
        .by_ref()
        .take(maximum as u64)
        .read_to_end(&mut body)
        .ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn retry_delay(response: &Response, attempt: u32, maximum: Duration) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
        .unwrap_or_else(|| exponential_delay(attempt, maximum))
        .min(maximum)
}

fn parse_retry_after(raw: &str) -> Option<Duration> {
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let deadline = httpdate::parse_http_date(raw).ok()?;
    Some(
        deadline
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

fn exponential_delay(attempt: u32, maximum: Duration) -> Duration {
    Duration::from_millis(250u64.saturating_mul(1u64 << attempt.min(16))).min(maximum)
}

pub(crate) fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme().eq_ignore_ascii_case(right.scheme())
        && left
            .host_str()
            .zip(right.host_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        && left.port_or_known_default() == right.port_or_known_default()
}

fn display_origin(url: &Url) -> String {
    match url.port() {
        Some(port) => format!(
            "{}://{}:{port}",
            url.scheme(),
            url.host_str().unwrap_or("?")
        ),
        None => format!("{}://{}", url.scheme(), url.host_str().unwrap_or("?")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_endpoint_to_base_path() {
        let base = Url::parse("https://example.test/api/v1").unwrap();
        assert_eq!(
            endpoint_url(&base, "/items").unwrap().as_str(),
            "https://example.test/api/v1/items"
        );
    }

    #[test]
    fn resolves_query_only_next_page() {
        let base = Url::parse("https://example.test/api").unwrap();
        let current = Url::parse("https://example.test/api/items?page=1").unwrap();
        assert_eq!(
            resolve_page_url(&base, &current, "?page=2", false)
                .unwrap()
                .as_str(),
            "https://example.test/api/items?page=2"
        );
    }

    #[test]
    fn blocks_cross_origin_pagination() {
        let base = Url::parse("https://example.test/api").unwrap();
        let current = Url::parse("https://example.test/api/items").unwrap();
        assert!(resolve_page_url(&base, &current, "https://attacker.test/collect", false).is_err());
    }

    #[test]
    fn accepts_json_and_structured_json_media_types() {
        assert!(is_json_content_type("application/json; charset=utf-8"));
        assert!(is_json_content_type("application/geo+json"));
        assert!(!is_json_content_type("text/html"));
    }

    #[test]
    fn retries_only_idempotent_methods_automatically() {
        assert!(automatically_retryable(&Method::GET));
        assert!(automatically_retryable(&Method::PUT));
        assert!(automatically_retryable(&Method::DELETE));
        assert!(!automatically_retryable(&Method::POST));
        assert!(!automatically_retryable(&Method::PATCH));
    }
}
