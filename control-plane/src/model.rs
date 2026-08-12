use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

pub const API_VERSION: &str = "openapi-fdw/v1";
pub const MANIFEST_KIND: &str = "OpenApiFdwBundle";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceDefinition {
    pub name: String,
    pub schema: String,
    #[serde(default = "default_remote_schema")]
    pub remote_schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default = "default_methods")]
    pub methods: Vec<String>,
    #[serde(default = "default_true")]
    pub include_attrs: bool,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub auth: AuthDefinition,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers_env: BTreeMap<String, String>,
    #[serde(default)]
    pub settings: ServerSettings,
}

impl SourceDefinition {
    pub fn validate(&self, require_credentials: bool) -> Result<(), String> {
        validate_identifier(&self.name, "source name")?;
        validate_identifier(&self.schema, "schema")?;
        validate_identifier(&self.remote_schema, "remote schema")?;

        match (&self.spec_url, &self.spec_json) {
            (Some(_), Some(_)) => {
                return Err("provide `specUrl` or `specJson`, not both".to_string());
            }
            (None, None) => {
                return Err("one of `specUrl` or `specJson` is required".to_string());
            }
            _ => {}
        }
        if let Some(spec_url) = &self.spec_url {
            validate_http_url(spec_url, "specUrl", self.settings.allow_http)?;
        }
        if self
            .spec_json
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err("`specJson` cannot be empty".to_string());
        }
        if let Some(base_url) = &self.base_url {
            let parsed = validate_http_url(base_url, "baseUrl", self.settings.allow_http)?;
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err("`baseUrl` cannot contain a query string or fragment".to_string());
            }
        }

        if self.methods.is_empty() {
            return Err("select at least one import method".to_string());
        }
        let mut methods = BTreeSet::new();
        for method in &self.methods {
            let normalized = method.trim().to_ascii_uppercase();
            if !matches!(normalized.as_str(), "GET" | "POST") {
                return Err(format!("unsupported method `{method}`; use GET or POST"));
            }
            if !methods.insert(normalized) {
                return Err(format!("method `{method}` is listed more than once"));
            }
        }
        for table in &self.tables {
            validate_identifier(table, "table")?;
        }
        if self.tables.iter().collect::<BTreeSet<_>>().len() != self.tables.len() {
            return Err("the selected table list contains duplicates".to_string());
        }

        for name in self.headers.keys().chain(self.headers_env.keys()) {
            validate_header_name(name)?;
        }
        for name in self.headers_env.values() {
            validate_environment_name(name, "header environment variable")?;
        }
        if self.headers.keys().any(|name| {
            self.headers_env
                .keys()
                .any(|other| name.eq_ignore_ascii_case(other))
        }) {
            return Err(
                "a header cannot be configured both literally and by environment".to_string(),
            );
        }
        if self.headers.values().any(|value| value == "[REDACTED]") && require_credentials {
            return Err("re-enter redacted literal header values before applying".to_string());
        }

        self.auth.validate(require_credentials)?;
        self.settings.validate()?;
        Ok(())
    }

    pub fn redacted(&self) -> Self {
        let mut safe = self.clone();
        for value in safe.headers.values_mut() {
            *value = "[REDACTED]".to_string();
        }
        safe.auth = safe.auth.redacted();
        safe
    }

    pub fn normalized_methods(&self) -> String {
        self.methods
            .iter()
            .map(|method| method.trim().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthDefinition {
    #[default]
    None,
    Bearer {
        secret: SecretValue,
    },
    ApiKey {
        secret: SecretValue,
        #[serde(default = "default_api_key_name")]
        name: String,
        #[serde(default)]
        location: ApiKeyLocation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
}

impl AuthDefinition {
    fn validate(&self, require_credentials: bool) -> Result<(), String> {
        match self {
            Self::None => Ok(()),
            Self::Bearer { secret } => secret.validate("bearer token", require_credentials),
            Self::ApiKey {
                secret,
                name,
                prefix,
                ..
            } => {
                secret.validate("API key", require_credentials)?;
                if name.trim().is_empty() || name.len() > 128 {
                    return Err("API-key name must contain 1 to 128 characters".to_string());
                }
                if prefix.as_ref().is_some_and(|value| value.len() > 64) {
                    return Err("API-key prefix cannot exceed 64 characters".to_string());
                }
                Ok(())
            }
        }
    }

    fn redacted(&self) -> Self {
        match self {
            Self::None => Self::None,
            Self::Bearer { secret } => Self::Bearer {
                secret: secret.redacted(),
            },
            Self::ApiKey {
                secret,
                name,
                location,
                prefix,
            } => Self::ApiKey {
                secret: secret.redacted(),
                name: name.clone(),
                location: *location,
                prefix: prefix.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub configured: bool,
}

impl SecretValue {
    fn validate(&self, label: &str, require_credentials: bool) -> Result<(), String> {
        let value = self.value.as_ref().filter(|value| !value.is_empty());
        let environment = self.env.as_ref().filter(|value| !value.is_empty());
        if value.is_some() && environment.is_some() {
            return Err(format!(
                "configure {label} as a literal or environment reference, not both"
            ));
        }
        if let Some(environment) = environment {
            validate_environment_name(environment, label)?;
        }
        if require_credentials && value.is_none() && environment.is_none() {
            let detail = if self.configured {
                "the exported value was redacted; re-enter it"
            } else {
                "a value or environment reference is required"
            };
            return Err(format!("{label}: {detail}"));
        }
        Ok(())
    }

    fn redacted(&self) -> Self {
        Self {
            value: None,
            env: self.env.clone(),
            configured: self.configured
                || self.value.as_ref().is_some_and(|value| !value.is_empty()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    #[default]
    Header,
    Query,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerSettings {
    #[serde(default)]
    pub allow_http: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spec_with_auth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_max_response")]
    pub max_response_bytes: u64,
    #[serde(default = "default_max_pages")]
    pub max_pages: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u64,
}

impl ServerSettings {
    fn validate(&self) -> Result<(), String> {
        bounded(self.connect_timeout_ms, 1, 300_000, "connectTimeoutMs")?;
        bounded(self.request_timeout_ms, 1, 3_600_000, "requestTimeoutMs")?;
        bounded(
            self.max_response_bytes,
            1,
            512 * 1024 * 1024,
            "maxResponseBytes",
        )?;
        bounded(self.max_pages, 1, 10_000, "maxPages")?;
        bounded(self.max_retries, 0, 10, "maxRetries")?;
        if self
            .user_agent
            .as_ref()
            .is_some_and(|value| value.len() > 512)
        {
            return Err("userAgent cannot exceed 512 characters".to_string());
        }
        Ok(())
    }
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            allow_http: false,
            spec_with_auth: false,
            user_agent: None,
            connect_timeout_ms: default_connect_timeout(),
            request_timeout_ms: default_request_timeout(),
            max_response_bytes: default_max_response(),
            max_pages: default_max_pages(),
            max_retries: default_max_retries(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bundle {
    pub api_version: String,
    pub kind: String,
    pub sources: Vec<SourceDefinition>,
}

impl Bundle {
    pub fn new(sources: Vec<SourceDefinition>) -> Self {
        Self {
            api_version: API_VERSION.to_string(),
            kind: MANIFEST_KIND.to_string(),
            sources,
        }
    }

    pub fn validate(&self, require_credentials: bool) -> Result<(), String> {
        if self.api_version != API_VERSION {
            return Err(format!("unsupported apiVersion `{}`", self.api_version));
        }
        if self.kind != MANIFEST_KIND {
            return Err(format!("unsupported manifest kind `{}`", self.kind));
        }
        if self.sources.is_empty() {
            return Err("the bundle has no sources".to_string());
        }
        let mut names = BTreeSet::new();
        for source in &self.sources {
            source.validate(require_credentials)?;
            if !names.insert(&source.name) {
                return Err(format!("source `{}` appears more than once", source.name));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyRequest {
    pub source: SourceDefinition,
    #[serde(default)]
    pub replace: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteRequest {
    pub confirm: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleQuery {
    #[serde(default = "default_sample_limit")]
    pub limit: u32,
    pub filter_column: Option<String>,
    pub filter_value: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub tables: Vec<TableState>,
    pub sql: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    pub name: String,
    pub managed: bool,
    pub definition: Option<SourceDefinition>,
    pub options: serde_json::Value,
    pub tables: Vec<TableState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableState {
    pub schema: String,
    pub name: String,
    pub endpoint: String,
    pub method: String,
    pub columns: Vec<ColumnState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnState {
    pub name: String,
    pub data_type: String,
    pub ordinal: i16,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlState {
    pub extension_version: Option<String>,
    pub postgres_version: String,
    pub sources: Vec<SourceState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResult {
    pub ok: bool,
    pub message: String,
    pub sql: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleResult {
    pub rows: Vec<serde_json::Value>,
    pub sql: String,
}

pub(crate) fn validate_identifier(value: &str, label: &str) -> Result<(), String> {
    let mut characters = value.chars();
    let first = characters.next();
    let valid = value.len() <= 63
        && first.is_some_and(|character| character == '_' || character.is_ascii_lowercase())
        && characters.all(|character| {
            character == '_' || character.is_ascii_lowercase() || character.is_ascii_digit()
        });
    if !valid {
        return Err(format!(
            "{label} `{value}` must be a lowercase PostgreSQL identifier of at most 63 bytes"
        ));
    }
    Ok(())
}

fn validate_environment_name(value: &str, label: &str) -> Result<(), String> {
    let mut characters = value.chars();
    let valid = value.len() <= 128
        && characters
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !valid {
        return Err(format!(
            "{label} `{value}` is not a valid environment-variable name"
        ));
    }
    Ok(())
}

fn validate_header_name(value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        });
    if !valid {
        return Err(format!("header name `{value}` is invalid"));
    }
    Ok(())
}

fn validate_http_url(value: &str, label: &str, allow_http: bool) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|_| format!("`{label}` is not an absolute URL"))?;
    match parsed.scheme() {
        "https" => {}
        "http" if allow_http => {}
        "http" => return Err(format!("`{label}` uses HTTP; enable allowHttp explicitly")),
        _ => return Err(format!("`{label}` must use HTTP or HTTPS")),
    }
    if parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!(
            "`{label}` must have a host and cannot embed credentials"
        ));
    }
    Ok(parsed)
}

fn bounded(value: u64, minimum: u64, maximum: u64, label: &str) -> Result<(), String> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(format!("{label} must be between {minimum} and {maximum}"))
    }
}

fn default_remote_schema() -> String {
    "api".to_string()
}

fn default_methods() -> Vec<String> {
    vec!["GET".to_string()]
}

fn default_api_key_name() -> String {
    "x-api-key".to_string()
}

fn default_true() -> bool {
    true
}

fn default_connect_timeout() -> u64 {
    5_000
}

fn default_request_timeout() -> u64 {
    30_000
}

fn default_max_response() -> u64 {
    50 * 1024 * 1024
}

fn default_max_pages() -> u64 {
    100
}

fn default_max_retries() -> u64 {
    2
}

fn default_sample_limit() -> u32 {
    20
}

fn is_false(value: &bool) -> bool {
    !value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> SourceDefinition {
        SourceDefinition {
            name: "brasilapi".to_string(),
            schema: "brasil".to_string(),
            remote_schema: "api".to_string(),
            spec_url: Some("https://example.test/openapi.json".to_string()),
            spec_json: None,
            base_url: None,
            methods: vec!["GET".to_string()],
            include_attrs: true,
            tables: vec!["banks".to_string()],
            auth: AuthDefinition::None,
            headers: BTreeMap::new(),
            headers_env: BTreeMap::new(),
            settings: ServerSettings::default(),
        }
    }

    #[test]
    fn validates_a_minimal_source() {
        assert!(example().validate(true).is_ok());
    }

    #[test]
    fn rejects_identifier_injection() {
        let mut source = example();
        source.schema = "public; DROP DATABASE postgres".to_string();
        assert!(source.validate(true).is_err());
    }

    #[test]
    fn redacts_literal_secrets_but_keeps_environment_references() {
        let mut source = example();
        source.auth = AuthDefinition::ApiKey {
            secret: SecretValue {
                value: Some("secret".to_string()),
                env: None,
                configured: false,
            },
            name: "x-key".to_string(),
            location: ApiKeyLocation::Header,
            prefix: None,
        };
        source
            .headers
            .insert("x-private".to_string(), "hidden".to_string());
        source
            .headers_env
            .insert("x-shared".to_string(), "SHARED_HEADER".to_string());
        let safe = source.redacted();
        assert_eq!(safe.headers["x-private"], "[REDACTED]");
        assert_eq!(safe.headers_env["x-shared"], "SHARED_HEADER");
        let AuthDefinition::ApiKey { secret, .. } = safe.auth else {
            panic!("expected API-key auth")
        };
        assert!(secret.value.is_none());
        assert!(secret.configured);
    }
}
