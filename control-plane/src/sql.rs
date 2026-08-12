use crate::model::{ApiKeyLocation, AuthDefinition, SourceDefinition};
use std::collections::BTreeMap;

pub struct SqlPlan {
    pub create_extension: String,
    pub drop_existing: Option<String>,
    pub create_server: String,
    pub create_schema: String,
    pub import_schema: String,
}

impl SqlPlan {
    pub fn statements(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.create_extension.as_str())
            .chain(self.drop_existing.as_deref())
            .chain(std::iter::once(self.create_server.as_str()))
            .chain(std::iter::once(self.create_schema.as_str()))
            .chain(std::iter::once(self.import_schema.as_str()))
    }

    pub fn display(&self) -> String {
        self.statements()
            .map(|statement| format!("{};", statement.trim_end_matches(';')))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

pub fn source_plan(
    source: &SourceDefinition,
    server_name: &str,
    schema_name: &str,
    replace: bool,
    redact: bool,
) -> Result<SqlPlan, String> {
    let create_server = create_server_sql(source, server_name, redact)?;
    Ok(SqlPlan {
        create_extension: "CREATE EXTENSION IF NOT EXISTS openapi_fdw".to_string(),
        drop_existing: replace.then(|| {
            format!(
                "DROP SERVER IF EXISTS {} CASCADE",
                quote_identifier(server_name)
            )
        }),
        create_server,
        create_schema: format!(
            "CREATE SCHEMA IF NOT EXISTS {}",
            quote_identifier(schema_name)
        ),
        import_schema: import_sql(source, server_name, schema_name),
    })
}

pub fn drop_server_sql(server_name: &str) -> String {
    format!("DROP SERVER {} CASCADE", quote_identifier(server_name))
}

pub fn sample_sql(
    schema: &str,
    table: &str,
    limit: u32,
    filter: Option<(&str, &str)>,
) -> Result<String, String> {
    let predicate = match filter {
        Some((column, value)) => format!(
            " WHERE {} = {}",
            quote_identifier(column),
            quote_literal(value)?
        ),
        None => String::new(),
    };
    Ok(format!(
        "SELECT * FROM {}.{}{} LIMIT {}",
        quote_identifier(schema),
        quote_identifier(table),
        predicate,
        limit
    ))
}

fn create_server_sql(
    source: &SourceDefinition,
    server_name: &str,
    redact: bool,
) -> Result<String, String> {
    let mut options = BTreeMap::<String, String>::new();
    if let Some(spec_url) = &source.spec_url {
        options.insert("spec_url".to_string(), spec_url.clone());
    }
    if let Some(spec_json) = &source.spec_json {
        options.insert(
            "spec_json".to_string(),
            if redact {
                format!("[inline OpenAPI document: {} bytes]", spec_json.len())
            } else {
                spec_json.clone()
            },
        );
    }
    if let Some(base_url) = &source.base_url {
        options.insert("base_url".to_string(), base_url.clone());
    }
    if !source.headers.is_empty() {
        let headers = if redact {
            source
                .headers
                .keys()
                .map(|name| (name.clone(), "[REDACTED]".to_string()))
                .collect::<BTreeMap<_, _>>()
        } else {
            source.headers.clone()
        };
        options.insert(
            "headers".to_string(),
            serde_json::to_string(&headers).map_err(|error| error.to_string())?,
        );
    }
    if !source.headers_env.is_empty() {
        options.insert(
            "headers_env".to_string(),
            serde_json::to_string(&source.headers_env).map_err(|error| error.to_string())?,
        );
    }

    match &source.auth {
        AuthDefinition::None => {}
        AuthDefinition::Bearer { secret } => {
            if let Some(environment) = &secret.env {
                options.insert("bearer_token_env".to_string(), environment.clone());
            } else if let Some(value) = &secret.value {
                options.insert(
                    "bearer_token".to_string(),
                    if redact {
                        "[REDACTED]".to_string()
                    } else {
                        value.clone()
                    },
                );
            }
        }
        AuthDefinition::ApiKey {
            secret,
            name,
            location,
            prefix,
        } => {
            if let Some(environment) = &secret.env {
                options.insert("api_key_env".to_string(), environment.clone());
            } else if let Some(value) = &secret.value {
                options.insert(
                    "api_key".to_string(),
                    if redact {
                        "[REDACTED]".to_string()
                    } else {
                        value.clone()
                    },
                );
            }
            options.insert("api_key_name".to_string(), name.clone());
            options.insert(
                "api_key_location".to_string(),
                match location {
                    ApiKeyLocation::Header => "header",
                    ApiKeyLocation::Query => "query",
                }
                .to_string(),
            );
            if let Some(prefix) = prefix.as_ref().filter(|value| !value.trim().is_empty()) {
                options.insert("api_key_prefix".to_string(), prefix.trim().to_string());
            }
        }
    }

    if source.settings.allow_http {
        options.insert("allow_http".to_string(), "true".to_string());
    }
    if source.settings.spec_with_auth {
        options.insert("spec_with_auth".to_string(), "true".to_string());
    }
    if let Some(user_agent) = source
        .settings
        .user_agent
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        options.insert("user_agent".to_string(), user_agent.clone());
    }
    options.insert(
        "connect_timeout_ms".to_string(),
        source.settings.connect_timeout_ms.to_string(),
    );
    options.insert(
        "request_timeout_ms".to_string(),
        source.settings.request_timeout_ms.to_string(),
    );
    options.insert(
        "max_response_bytes".to_string(),
        source.settings.max_response_bytes.to_string(),
    );
    options.insert(
        "max_pages".to_string(),
        source.settings.max_pages.to_string(),
    );
    options.insert(
        "max_retries".to_string(),
        source.settings.max_retries.to_string(),
    );

    let rendered = options
        .into_iter()
        .map(|(name, value)| quote_literal(&value).map(|value| format!("    {name} {value}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "CREATE SERVER {}\n  FOREIGN DATA WRAPPER openapi_fdw\n  OPTIONS (\n{}\n  )",
        quote_identifier(server_name),
        rendered.join(",\n")
    ))
}

fn import_sql(source: &SourceDefinition, server_name: &str, schema_name: &str) -> String {
    let selection = if source.tables.is_empty() {
        String::new()
    } else {
        format!(
            "\n  LIMIT TO ({})",
            source
                .tables
                .iter()
                .map(|table| quote_identifier(table))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "IMPORT FOREIGN SCHEMA {}{}\n  FROM SERVER {}\n  INTO {}\n  OPTIONS (methods {}, include_attrs {})",
        quote_identifier(&source.remote_schema),
        selection,
        quote_identifier(server_name),
        quote_identifier(schema_name),
        quote_literal(&source.normalized_methods()).expect("methods cannot contain NUL"),
        quote_literal(if source.include_attrs {
            "true"
        } else {
            "false"
        })
        .expect("boolean cannot contain NUL")
    )
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub fn quote_literal(value: &str) -> Result<String, String> {
    if value.contains('\0') {
        return Err("PostgreSQL strings cannot contain NUL bytes".to_string());
    }
    Ok(format!(
        "E'{}'",
        value.replace('\\', "\\\\").replace('\'', "''")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SecretValue, ServerSettings};
    use std::collections::BTreeMap;

    fn source() -> SourceDefinition {
        SourceDefinition {
            name: "vendor".to_string(),
            schema: "vendor".to_string(),
            remote_schema: "api".to_string(),
            spec_url: Some("https://example.test/openapi.json".to_string()),
            spec_json: None,
            base_url: None,
            methods: vec!["GET".to_string()],
            include_attrs: true,
            tables: vec!["items".to_string()],
            auth: AuthDefinition::Bearer {
                secret: SecretValue {
                    value: Some("do-not-show".to_string()),
                    env: None,
                    configured: false,
                },
            },
            headers: BTreeMap::new(),
            headers_env: BTreeMap::new(),
            settings: ServerSettings::default(),
        }
    }

    #[test]
    fn generated_preview_redacts_credentials() {
        let plan = source_plan(&source(), "vendor", "vendor", false, true).unwrap();
        let display = plan.display();
        assert!(display.contains("[REDACTED]"));
        assert!(!display.contains("do-not-show"));
    }

    #[test]
    fn quotes_postgres_literals_and_identifiers() {
        assert_eq!(quote_identifier("odd\"name"), "\"odd\"\"name\"");
        assert_eq!(quote_literal("a'b\\c").unwrap(), "E'a''b\\\\c'");
        assert!(quote_literal("bad\0value").is_err());
    }
}
