use crate::error::{OpenApiFdwError, Result};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;
use url::Url;

pub(crate) const DEFAULT_MAX_RESPONSE_BYTES: usize = 50 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_PAGES: usize = 100;

#[derive(Clone)]
pub(crate) enum Auth {
    None,
    Header { name: String, value: String },
    Query { name: String, value: String },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Header { name, .. } => formatter
                .debug_struct("Header")
                .field("name", name)
                .field("value", &"[REDACTED]")
                .finish(),
            Self::Query { name, .. } => formatter
                .debug_struct("Query")
                .field("name", name)
                .field("value", &"[REDACTED]")
                .finish(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ServerConfig {
    pub(crate) base_url: Option<Url>,
    pub(crate) spec_url: Option<Url>,
    pub(crate) spec_json: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) auth: Auth,
    pub(crate) allow_http: bool,
    pub(crate) allow_cross_origin_pagination: bool,
    pub(crate) connect_timeout: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) max_response_bytes: usize,
    pub(crate) max_pages: usize,
    pub(crate) max_retries: u32,
    pub(crate) max_retry_delay: Duration,
    pub(crate) max_redirects: usize,
    pub(crate) spec_with_auth: bool,
    secrets: Vec<String>,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerConfig")
            .field("base_url", &self.base_url)
            .field("spec_url", &self.spec_url)
            .field(
                "spec_json",
                &self.spec_json.as_ref().map(|value| value.len()),
            )
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("auth", &self.auth)
            .field("allow_http", &self.allow_http)
            .field(
                "allow_cross_origin_pagination",
                &self.allow_cross_origin_pagination,
            )
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_pages", &self.max_pages)
            .field("max_retries", &self.max_retries)
            .field("max_retry_delay", &self.max_retry_delay)
            .field("max_redirects", &self.max_redirects)
            .field("spec_with_auth", &self.spec_with_auth)
            .finish()
    }
}

impl ServerConfig {
    pub(crate) fn from_options(options: &HashMap<String, String>) -> Result<Self> {
        let allow_http = parse_bool(options, "allow_http", false)?;
        let base_url = options
            .get("base_url")
            .map(|value| validate_url(value, "base_url", allow_http))
            .transpose()?;
        let spec_url = options
            .get("spec_url")
            .map(|value| validate_url(value, "spec_url", allow_http))
            .transpose()?;
        let spec_json = options.get("spec_json").cloned();

        if base_url.is_none() && spec_url.is_none() && spec_json.is_none() {
            return Err(OpenApiFdwError::Configuration(
                "one of `base_url`, `spec_url`, or `spec_json` is required".to_string(),
            ));
        }
        if spec_url.is_some() && spec_json.is_some() {
            return Err(OpenApiFdwError::Configuration(
                "`spec_url` and `spec_json` are mutually exclusive".to_string(),
            ));
        }

        let mut headers = BTreeMap::from([
            (
                "user-agent".to_string(),
                options
                    .get("user_agent")
                    .cloned()
                    .unwrap_or_else(|| "openapi_fdw/0.3".to_string()),
            ),
            (
                "accept".to_string(),
                options
                    .get("accept")
                    .cloned()
                    .unwrap_or_else(|| "application/json".to_string()),
            ),
        ]);
        let mut header_secrets = Vec::new();
        let mut custom_header_names = BTreeSet::new();
        if let Some(raw) = options.get("headers") {
            for (name, value) in parse_string_map(raw, "headers")? {
                let normalized = name.to_ascii_lowercase();
                validate_custom_header(&normalized, &name)?;
                // A custom header can carry an API-specific credential even
                // when its name does not look sensitive. Redact every custom
                // value from remote error excerpts.
                header_secrets.push(value.clone());
                custom_header_names.insert(normalized.clone());
                headers.insert(normalized, value);
            }
        }

        if let Some(raw) = options.get("headers_env") {
            for (name, environment_name) in parse_string_map(raw, "headers_env")? {
                let normalized = name.to_ascii_lowercase();
                validate_custom_header(&normalized, &name)?;
                if custom_header_names.contains(&normalized) {
                    return Err(OpenApiFdwError::Configuration(format!(
                        "header `{name}` is configured by both `headers` and `headers_env`"
                    )));
                }
                let value = environment_secret(&environment_name, "headers_env")?;
                header_secrets.push(value.clone());
                custom_header_names.insert(normalized.clone());
                headers.insert(normalized, value);
            }
        }

        let api_key = secret_option(options, "api_key", "api_key_env")?;
        let bearer = secret_option(options, "bearer_token", "bearer_token_env")?;
        if api_key.is_some() && bearer.is_some() {
            return Err(OpenApiFdwError::Configuration(
                "configure either `api_key` or `bearer_token`, not both".to_string(),
            ));
        }

        let auth = if let Some(token) = &bearer {
            Auth::Header {
                name: "authorization".to_string(),
                value: format!("Bearer {token}"),
            }
        } else if let Some(key) = &api_key {
            let name = options
                .get("api_key_name")
                .cloned()
                .unwrap_or_else(|| "x-api-key".to_string());
            let value = match options.get("api_key_prefix") {
                Some(prefix) if !prefix.trim().is_empty() => format!("{} {key}", prefix.trim()),
                _ => key.clone(),
            };
            match options.get("api_key_location").map(String::as_str) {
                None | Some("header") => Auth::Header { name, value },
                Some("query") => Auth::Query { name, value },
                Some(other) => {
                    return Err(OpenApiFdwError::Configuration(format!(
                        "unsupported api_key_location `{other}`; use `header` or `query`"
                    )));
                }
            }
        } else {
            Auth::None
        };
        if let Auth::Header { name, .. } = &auth
            && headers.contains_key(&name.to_ascii_lowercase())
        {
            return Err(OpenApiFdwError::Configuration(format!(
                "authentication header `{name}` is also configured as a custom or built-in header"
            )));
        }

        let mut secrets = header_secrets;
        secrets.extend(api_key);
        secrets.extend(bearer);
        if let Auth::Header { value, .. } | Auth::Query { value, .. } = &auth {
            secrets.push(value.clone());
        }
        secrets.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        secrets.dedup();

        Ok(Self {
            base_url,
            spec_url,
            spec_json,
            headers,
            auth,
            allow_http,
            allow_cross_origin_pagination: parse_bool(
                options,
                "allow_cross_origin_pagination",
                false,
            )?,
            connect_timeout: Duration::from_millis(parse_bounded(
                options,
                "connect_timeout_ms",
                5_000,
                1,
                300_000,
            )?),
            request_timeout: Duration::from_millis(parse_bounded(
                options,
                "request_timeout_ms",
                30_000,
                1,
                3_600_000,
            )?),
            max_response_bytes: parse_bounded(
                options,
                "max_response_bytes",
                DEFAULT_MAX_RESPONSE_BYTES as u64,
                1,
                512 * 1024 * 1024,
            )? as usize,
            max_pages: parse_bounded(options, "max_pages", DEFAULT_MAX_PAGES as u64, 1, 10_000)?
                as usize,
            max_retries: parse_bounded(options, "max_retries", 2, 0, 10)? as u32,
            max_retry_delay: Duration::from_millis(parse_bounded(
                options,
                "max_retry_delay_ms",
                5_000,
                0,
                60_000,
            )?),
            max_redirects: parse_bounded(options, "max_redirects", 5, 0, 10)? as usize,
            spec_with_auth: parse_bool(options, "spec_with_auth", false)?,
            secrets,
        })
    }

    pub(crate) fn for_spec_fetch(&self) -> Self {
        if self.spec_with_auth {
            return self.clone();
        }
        let mut safe = self.clone();
        safe.headers
            .retain(|name, _| matches!(name.as_str(), "user-agent" | "accept"));
        safe.auth = Auth::None;
        safe.secrets.clear();
        safe
    }

    pub(crate) fn redact(&self, text: &str) -> String {
        self.secrets.iter().fold(text.to_string(), |safe, secret| {
            if secret.is_empty() {
                safe
            } else {
                safe.replace(secret, "[REDACTED]")
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaginationMode {
    Auto,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TypeErrorMode {
    Error,
    Null,
}

#[derive(Clone, Debug)]
pub(crate) struct TableConfig {
    pub(crate) endpoint: String,
    pub(crate) method: String,
    pub(crate) response_path: Option<String>,
    pub(crate) object_path: Option<String>,
    pub(crate) query_params: Map<String, Value>,
    pub(crate) request_body: Option<Value>,
    pub(crate) column_map: HashMap<String, String>,
    pub(crate) query_param_map: HashMap<String, String>,
    pub(crate) attrs_column: String,
    pub(crate) limit_param: Option<String>,
    pub(crate) page_size: Option<usize>,
    pub(crate) page_size_param: Option<String>,
    pub(crate) cursor_path: Option<String>,
    pub(crate) cursor_param: Option<String>,
    pub(crate) pagination: PaginationMode,
    pub(crate) max_pages: Option<usize>,
    pub(crate) type_error: TypeErrorMode,
}

impl TableConfig {
    pub(crate) fn from_options(options: &HashMap<String, String>) -> Result<Self> {
        let endpoint = options
            .get("endpoint")
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| {
                OpenApiFdwError::Configuration(
                    "foreign table option `endpoint` is required".to_string(),
                )
            })?;
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            return Err(OpenApiFdwError::Configuration(
                "`endpoint` must be relative to the server `base_url`".to_string(),
            ));
        }

        let method = options
            .get("method")
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_else(|| "GET".to_string());
        if method != "GET" && method != "POST" {
            return Err(OpenApiFdwError::Configuration(format!(
                "unsupported method `{method}`; read scans support GET and POST"
            )));
        }

        let response_path = parse_pointer(options.get("response_path"), "response_path")?;
        let object_path = parse_pointer(options.get("object_path"), "object_path")?;
        let cursor_path = parse_pointer(options.get("cursor_path"), "cursor_path")?;

        let query_params = match options.get("query_params") {
            Some(raw) => parse_json_object(raw, "query_params")?,
            None => Map::new(),
        };
        let request_body = options
            .get("request_body")
            .map(|raw| parse_json(raw, "request_body"))
            .transpose()?;

        let pagination = match options.get("pagination").map(String::as_str) {
            None | Some("auto") => PaginationMode::Auto,
            Some("none") => PaginationMode::None,
            Some(other) => {
                return Err(OpenApiFdwError::Configuration(format!(
                    "unsupported pagination mode `{other}`; use `auto` or `none`"
                )));
            }
        };
        let type_error = match options.get("on_type_error").map(String::as_str) {
            None | Some("error") => TypeErrorMode::Error,
            Some("null") => TypeErrorMode::Null,
            Some(other) => {
                return Err(OpenApiFdwError::Configuration(format!(
                    "unsupported on_type_error value `{other}`; use `error` or `null`"
                )));
            }
        };

        Ok(Self {
            endpoint,
            method,
            response_path,
            object_path,
            query_params,
            request_body,
            column_map: options
                .get("column_map")
                .map(|raw| parse_string_map(raw, "column_map"))
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .collect(),
            query_param_map: options
                .get("query_param_map")
                .map(|raw| parse_string_map(raw, "query_param_map"))
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .collect(),
            attrs_column: options
                .get("attrs_column")
                .cloned()
                .unwrap_or_else(|| "attrs".to_string()),
            limit_param: non_empty_option(options, "limit_param", Some("limit")),
            page_size: parse_optional_bounded(options, "page_size", 1, 1_000_000)?
                .map(|value| value as usize),
            page_size_param: non_empty_option(options, "page_size_param", None),
            cursor_path,
            cursor_param: non_empty_option(options, "cursor_param", Some("cursor")),
            pagination,
            max_pages: parse_optional_bounded(options, "max_pages", 1, 10_000)?
                .map(|value| value as usize),
            type_error,
        })
    }
}

pub(crate) fn validate_url(raw: &str, label: &str, allow_http: bool) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| {
        OpenApiFdwError::Configuration(format!("`{label}` is not a valid absolute URL"))
    })?;
    match url.scheme() {
        "https" => {}
        "http" if allow_http => {}
        "http" => {
            return Err(OpenApiFdwError::Configuration(format!(
                "`{label}` uses plain HTTP; set `allow_http` to `true` for trusted local services"
            )));
        }
        _ => {
            return Err(OpenApiFdwError::Configuration(format!(
                "`{label}` must use HTTP or HTTPS"
            )));
        }
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err(OpenApiFdwError::Configuration(format!(
            "`{label}` must have a host and must not contain embedded credentials"
        )));
    }
    if label == "base_url" && (url.query().is_some() || url.fragment().is_some()) {
        return Err(OpenApiFdwError::Configuration(
            "`base_url` must not contain a query string or fragment".to_string(),
        ));
    }
    Ok(url)
}

fn parse_bool(options: &HashMap<String, String>, name: &str, default: bool) -> Result<bool> {
    match options.get(name).map(String::as_str) {
        None => Ok(default),
        Some("true" | "on" | "yes" | "1") => Ok(true),
        Some("false" | "off" | "no" | "0") => Ok(false),
        Some(_) => Err(OpenApiFdwError::Configuration(format!(
            "option `{name}` must be a boolean"
        ))),
    }
}

fn parse_bounded(
    options: &HashMap<String, String>,
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64> {
    parse_optional_bounded(options, name, minimum, maximum).map(|value| value.unwrap_or(default))
}

fn parse_optional_bounded(
    options: &HashMap<String, String>,
    name: &str,
    minimum: u64,
    maximum: u64,
) -> Result<Option<u64>> {
    options
        .get(name)
        .map(|raw| {
            raw.parse::<u64>()
                .ok()
                .filter(|value| (*value >= minimum) && (*value <= maximum))
                .ok_or_else(|| {
                    OpenApiFdwError::Configuration(format!(
                        "option `{name}` must be between {minimum} and {maximum}"
                    ))
                })
        })
        .transpose()
}

fn parse_pointer(raw: Option<&String>, name: &str) -> Result<Option<String>> {
    raw.filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with('/') {
                Ok(value.clone())
            } else {
                Err(OpenApiFdwError::Configuration(format!(
                    "option `{name}` must be an RFC 6901 JSON Pointer beginning with `/`"
                )))
            }
        })
        .transpose()
}

fn parse_json(raw: &str, name: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|error| {
        OpenApiFdwError::Configuration(format!("option `{name}` is invalid JSON: {error}"))
    })
}

fn parse_json_object(raw: &str, name: &str) -> Result<Map<String, Value>> {
    parse_json(raw, name)?.as_object().cloned().ok_or_else(|| {
        OpenApiFdwError::Configuration(format!("option `{name}` must be a JSON object"))
    })
}

fn parse_string_map(raw: &str, name: &str) -> Result<BTreeMap<String, String>> {
    parse_json_object(raw, name)?
        .into_iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or_else(|| {
                    OpenApiFdwError::Configuration(format!(
                        "option `{name}` value for `{key}` must be a string"
                    ))
                })
        })
        .collect()
}

fn validate_custom_header(normalized: &str, original: &str) -> Result<()> {
    if matches!(
        normalized,
        "host" | "content-length" | "connection" | "transfer-encoding"
    ) {
        return Err(OpenApiFdwError::Configuration(format!(
            "header `{original}` is controlled by the HTTP client"
        )));
    }
    Ok(())
}

fn secret_option(
    options: &HashMap<String, String>,
    literal_name: &str,
    environment_option: &str,
) -> Result<Option<String>> {
    let literal = options
        .get(literal_name)
        .filter(|value| !value.is_empty())
        .cloned();
    let environment_name = options
        .get(environment_option)
        .filter(|value| !value.is_empty());
    if literal.is_some() && environment_name.is_some() {
        return Err(OpenApiFdwError::Configuration(format!(
            "configure either `{literal_name}` or `{environment_option}`, not both"
        )));
    }
    environment_name
        .map(|name| environment_secret(name, environment_option))
        .transpose()
        .map(|value| value.or(literal))
}

fn environment_secret(environment_name: &str, option_name: &str) -> Result<String> {
    let valid = environment_name.len() <= 128
        && environment_name
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && environment_name
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !valid {
        return Err(OpenApiFdwError::Configuration(format!(
            "option `{option_name}` must name an environment variable"
        )));
    }
    match std::env::var(environment_name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            Err(OpenApiFdwError::Configuration(format!(
                "environment variable `{environment_name}` configured by `{option_name}` is missing or empty"
            )))
        }
        Err(std::env::VarError::NotUnicode(_)) => Err(OpenApiFdwError::Configuration(format!(
            "environment variable `{environment_name}` configured by `{option_name}` is not UTF-8"
        ))),
    }
}

fn non_empty_option(
    options: &HashMap<String, String>,
    name: &str,
    default: Option<&str>,
) -> Option<String> {
    match options.get(name) {
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(value.clone()),
        None => default.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_plain_http_by_default() {
        let options = HashMap::from([("base_url".to_string(), "http://example.test".to_string())]);
        assert!(ServerConfig::from_options(&options).is_err());
    }

    #[test]
    fn parses_jsonb_first_table_defaults() {
        let options = HashMap::from([("endpoint".to_string(), "/items".to_string())]);
        let config = TableConfig::from_options(&options).unwrap();
        assert_eq!(config.method, "GET");
        assert_eq!(config.attrs_column, "attrs");
        assert_eq!(config.limit_param.as_deref(), Some("limit"));
        assert_eq!(config.pagination, PaginationMode::Auto);
    }

    #[test]
    fn debug_output_never_contains_credentials() {
        let options = HashMap::from([
            ("base_url".to_string(), "https://example.test".to_string()),
            ("bearer_token".to_string(), "extremely-secret".to_string()),
            (
                "headers".to_string(),
                r#"{"x-vendor-auth":"vendor-secret"}"#.to_string(),
            ),
        ]);
        let config = ServerConfig::from_options(&options).unwrap();
        assert!(!format!("{config:?}").contains("extremely-secret"));
        assert_eq!(
            config.redact("failed for Bearer extremely-secret and vendor-secret"),
            "failed for [REDACTED] and [REDACTED]"
        );
    }

    #[test]
    fn missing_environment_secret_names_are_safe_to_report() {
        let options = HashMap::from([
            ("base_url".to_string(), "https://example.test".to_string()),
            (
                "bearer_token_env".to_string(),
                "OPENAPI_FDW_TEST_DEFINITELY_MISSING".to_string(),
            ),
        ]);
        let error = ServerConfig::from_options(&options)
            .unwrap_err()
            .to_string();
        assert!(error.contains("OPENAPI_FDW_TEST_DEFINITELY_MISSING"));
        assert!(error.contains("missing or empty"));
    }
}
