use crate::error::{OpenApiFdwError, Result};
use crate::options::validate_url;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use url::Url;

const ENVELOPE_KEYS: &[&str] = &[
    "data", "results", "items", "records", "entries", "features", "@graph",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportedColumn {
    pub(crate) name: String,
    pub(crate) json_name: String,
    pub(crate) pg_type: &'static str,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedEndpoint {
    pub(crate) table_name: String,
    pub(crate) endpoint: String,
    pub(crate) method: String,
    pub(crate) response_path: Option<String>,
    pub(crate) object_path: Option<String>,
    pub(crate) columns: Vec<ImportedColumn>,
}

impl ImportedEndpoint {
    pub(crate) fn create_sql(
        &self,
        local_schema: &str,
        server_name: &str,
        include_attrs: bool,
    ) -> String {
        let mut columns = self.columns.clone();
        let mut attrs_ordinal = 0usize;
        let attrs_name = loop {
            let candidate = match attrs_ordinal {
                0 => "attrs".to_string(),
                1 => "_attrs".to_string(),
                value => format!("_attrs_{value}"),
            };
            if !columns.iter().any(|column| column.name == candidate) {
                break candidate;
            }
            attrs_ordinal += 1;
        };
        let mut definitions = columns
            .iter()
            .map(|column| format!("    {} {}", quote_ident(&column.name), column.pg_type))
            .collect::<Vec<_>>();
        if include_attrs {
            definitions.push(format!("    {} jsonb", quote_ident(&attrs_name)));
        }
        if definitions.is_empty() {
            definitions.push(format!("    {} jsonb", quote_ident(&attrs_name)));
        }

        let column_map = columns
            .drain(..)
            .filter(|column| column.name != column.json_name)
            .map(|column| (column.name, Value::String(column.json_name)))
            .collect::<Map<_, _>>();
        let mut options = vec![format!("    endpoint {}", quote_literal(&self.endpoint))];
        if self.method != "GET" {
            options.push(format!("    method {}", quote_literal(&self.method)));
        }
        if let Some(path) = &self.response_path {
            options.push(format!("    response_path {}", quote_literal(path)));
        }
        if let Some(path) = &self.object_path {
            options.push(format!("    object_path {}", quote_literal(path)));
        }
        if !column_map.is_empty() {
            options.push(format!(
                "    column_map {}",
                quote_literal(&Value::Object(column_map).to_string())
            ));
        }
        if include_attrs && attrs_name != "attrs" {
            options.push(format!("    attrs_column {}", quote_literal(&attrs_name)));
        }

        format!(
            "CREATE FOREIGN TABLE {}.{} (\n{}\n)\nSERVER {} OPTIONS (\n{}\n)",
            quote_ident(local_schema),
            quote_ident(&self.table_name),
            definitions.join(",\n"),
            quote_ident(server_name),
            options.join(",\n")
        )
    }
}

pub(crate) fn validate_spec(spec: &Value) -> Result<()> {
    let version = spec
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or_else(|| OpenApiFdwError::Spec("document has no `openapi` version".to_string()))?;
    if !version.starts_with("3.") {
        return Err(OpenApiFdwError::Spec(format!(
            "unsupported OpenAPI version `{version}`; only 3.0 and 3.1 are supported"
        )));
    }
    if !spec.get("paths").is_some_and(Value::is_object) {
        return Err(OpenApiFdwError::Spec(
            "document has no OpenAPI `paths` object".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn base_url_from_spec(
    spec: &Value,
    spec_url: Option<&Url>,
    allow_http: bool,
) -> Result<Url> {
    validate_spec(spec)?;
    let server = spec
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(Value::as_object)
        .ok_or_else(|| {
            OpenApiFdwError::Spec(
                "base_url is absent and the OpenAPI document has no server URL".to_string(),
            )
        })?;
    let mut raw = server
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| OpenApiFdwError::Spec("OpenAPI server has no URL".to_string()))?
        .to_string();
    if let Some(variables) = server.get("variables").and_then(Value::as_object) {
        for (name, variable) in variables {
            if let Some(default) = variable.get("default").and_then(value_as_string) {
                raw = raw.replace(&format!("{{{name}}}"), &default);
            }
        }
    }

    let absolute = match Url::parse(&raw) {
        Ok(url) => url,
        Err(_) => spec_url
            .ok_or_else(|| {
                OpenApiFdwError::Spec(
                    "relative OpenAPI server URL requires a `spec_url` base".to_string(),
                )
            })?
            .join(&raw)
            .map_err(|_| OpenApiFdwError::Spec("invalid relative server URL".to_string()))?,
    };
    validate_url(absolute.as_str(), "OpenAPI server URL", allow_http)
}

pub(crate) fn endpoints(spec: &Value, methods: &HashSet<String>) -> Result<Vec<ImportedEndpoint>> {
    validate_spec(spec)?;
    let paths = spec["paths"].as_object().expect("validated paths object");
    let mut imported = Vec::new();
    let mut used_names: HashMap<String, usize> = HashMap::new();

    let mut sorted_paths = paths.iter().collect::<Vec<_>>();
    sorted_paths.sort_by_key(|(path, _)| *path);
    for (path, raw_path_item) in sorted_paths {
        let path_item = resolve_ref(spec, raw_path_item, 0, &mut HashSet::new());
        let Some(path_object) = path_item.as_object() else {
            continue;
        };
        for method in ["get", "post"] {
            let method_upper = method.to_ascii_uppercase();
            if !methods.contains(&method_upper) {
                continue;
            }
            let Some(operation) = path_object.get(method) else {
                continue;
            };
            let operation = resolve_ref(spec, operation, 0, &mut HashSet::new());
            let Some(operation_object) = operation.as_object() else {
                continue;
            };
            let schema = response_schema(spec, operation_object);
            let (row_schema, response_path, object_path) = schema
                .as_ref()
                .map(|schema| row_schema(spec, schema))
                .unwrap_or_else(|| (json!({}), None, None));
            let mut columns = columns_from_schema(spec, &row_schema);
            let parameter_columns = path_parameter_columns(
                spec,
                path_object.get("parameters"),
                operation_object.get("parameters"),
                &columns,
            );
            columns.extend(parameter_columns);
            columns.sort_by(
                |left, right| match (left.name.as_str(), right.name.as_str()) {
                    ("id", "id") => std::cmp::Ordering::Equal,
                    ("id", _) => std::cmp::Ordering::Less,
                    (_, "id") => std::cmp::Ordering::Greater,
                    _ => left.name.cmp(&right.name),
                },
            );

            let raw_name = operation_object
                .get("operationId")
                .and_then(Value::as_str)
                .map(sanitize_identifier)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| table_name_from_path(path, &method_upper));
            let table_name = unique_identifier(raw_name, "endpoint", &mut used_names);

            imported.push(ImportedEndpoint {
                table_name,
                endpoint: path.clone(),
                method: method_upper,
                response_path,
                object_path,
                columns,
            });
        }
    }
    Ok(imported)
}

fn response_schema(spec: &Value, operation: &Map<String, Value>) -> Option<Value> {
    let responses = operation.get("responses")?.as_object()?;
    let mut keys = responses.keys().collect::<Vec<_>>();
    keys.sort();
    let response = ["200", "201", "202", "2XX", "default"]
        .iter()
        .find_map(|key| responses.get(*key))
        .or_else(|| {
            keys.into_iter()
                .find(|key| key.starts_with('2'))
                .and_then(|key| responses.get(key))
        })?;
    let response = resolve_ref(spec, response, 0, &mut HashSet::new());
    let content = response.get("content")?.as_object()?;
    let media = content
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("application/json") || name.ends_with("+json"))
        .or_else(|| content.iter().next())?
        .1;
    media.get("schema").cloned()
}

fn row_schema(spec: &Value, raw_schema: &Value) -> (Value, Option<String>, Option<String>) {
    let schema = resolve_schema(spec, raw_schema, 0, &mut HashSet::new());
    if schema_type(&schema) == Some("array") {
        let item = schema
            .get("items")
            .map(|item| resolve_schema(spec, item, 0, &mut HashSet::new()))
            .unwrap_or_else(|| json!({}));
        return (item, None, None);
    }

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for key in ENVELOPE_KEYS {
            let Some(wrapper) = properties.get(*key) else {
                continue;
            };
            let wrapper = resolve_schema(spec, wrapper, 0, &mut HashSet::new());
            let row = if schema_type(&wrapper) == Some("array") {
                wrapper
                    .get("items")
                    .map(|item| resolve_schema(spec, item, 0, &mut HashSet::new()))
                    .unwrap_or_else(|| json!({}))
            } else if schema_type(&wrapper) == Some("object") {
                wrapper
            } else {
                continue;
            };
            if *key == "features"
                && let Some(feature_properties) = row
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get("properties"))
            {
                let feature_properties =
                    resolve_schema(spec, feature_properties, 0, &mut HashSet::new());
                if schema_type(&feature_properties) == Some("object") {
                    return (
                        feature_properties,
                        Some("/features".to_string()),
                        Some("/properties".to_string()),
                    );
                }
            }
            return (row, Some(format!("/{key}")), None);
        }
    }
    (schema, None, None)
}

fn columns_from_schema(spec: &Value, raw_schema: &Value) -> Vec<ImportedColumn> {
    let schema = resolve_schema(spec, raw_schema, 0, &mut HashSet::new());
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut properties = properties.iter().collect::<Vec<_>>();
    properties.sort_by_key(|(name, _)| *name);
    let mut seen = HashMap::<String, usize>::new();
    properties
        .into_iter()
        .filter(|(_, schema)| {
            !schema
                .get("writeOnly")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|(json_name, schema)| {
            let name = unique_identifier(sanitize_identifier(json_name), "field", &mut seen);
            ImportedColumn {
                name,
                json_name: json_name.clone(),
                pg_type: postgres_type(&resolve_schema(spec, schema, 0, &mut HashSet::new())),
            }
        })
        .collect()
}

fn path_parameter_columns(
    spec: &Value,
    path_parameters: Option<&Value>,
    operation_parameters: Option<&Value>,
    existing: &[ImportedColumn],
) -> Vec<ImportedColumn> {
    let mut columns = BTreeMap::<String, ImportedColumn>::new();
    for parameters in [path_parameters, operation_parameters]
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
    {
        for parameter in parameters {
            let parameter = resolve_ref(spec, parameter, 0, &mut HashSet::new());
            if parameter.get("in").and_then(Value::as_str) != Some("path") {
                continue;
            }
            let Some(json_name) = parameter.get("name").and_then(Value::as_str) else {
                continue;
            };
            let name = match sanitize_identifier(json_name) {
                name if name.is_empty() => "parameter".to_string(),
                name => name,
            };
            if existing.iter().any(|column| column.name == name) {
                continue;
            }
            let pg_type = parameter
                .get("schema")
                .map(|schema| postgres_type(&resolve_schema(spec, schema, 0, &mut HashSet::new())))
                .unwrap_or("text");
            columns.insert(
                name.clone(),
                ImportedColumn {
                    name,
                    json_name: json_name.to_string(),
                    pg_type,
                },
            );
        }
    }
    columns.into_values().collect()
}

fn postgres_type(schema: &Value) -> &'static str {
    match schema_type(schema) {
        Some("string") => match schema.get("format").and_then(Value::as_str) {
            Some("date") => "date",
            Some("date-time") => "timestamptz",
            Some("time") => "time",
            Some("uuid") => "uuid",
            _ => "text",
        },
        Some("integer") => match schema.get("format").and_then(Value::as_str) {
            Some("int32") => "integer",
            _ => "bigint",
        },
        Some("number") => match schema.get("format").and_then(Value::as_str) {
            Some("float") => "real",
            _ => "double precision",
        },
        Some("boolean") => "boolean",
        Some("array") => match schema.get("items").map(postgres_type) {
            Some("boolean") => "boolean[]",
            Some("integer") => "integer[]",
            Some("bigint") => "bigint[]",
            Some("real") => "real[]",
            Some("double precision") => "double precision[]",
            Some("text") => "text[]",
            _ => "jsonb",
        },
        _ => "jsonb",
    }
}

fn schema_type(schema: &Value) -> Option<&str> {
    let explicit = match schema.get("type") {
        Some(Value::String(value)) => Some(value.as_str()),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .find(|value| *value != "null"),
        _ => None,
    };
    explicit.or_else(|| {
        schema
            .get("properties")
            .filter(|value| value.is_object())
            .map(|_| "object")
    })
}

fn resolve_schema(spec: &Value, schema: &Value, depth: usize, seen: &mut HashSet<String>) -> Value {
    if depth >= 32 {
        return json!({});
    }
    let mut resolved = resolve_ref(spec, schema, depth, seen);
    let Some(object) = resolved.as_object_mut() else {
        return resolved;
    };

    if let Some(all_of) = object
        .remove("allOf")
        .and_then(|value| value.as_array().cloned())
    {
        let mut merged = Map::new();
        for member in all_of {
            merge_schema_objects(
                &mut merged,
                resolve_schema(spec, &member, depth + 1, seen),
                false,
            );
        }
        merge_schema_objects(&mut merged, Value::Object(object.clone()), false);
        resolved = Value::Object(merged);
    }

    let Some(object) = resolved.as_object_mut() else {
        return resolved;
    };
    for keyword in ["oneOf", "anyOf"] {
        if let Some(members) = object
            .remove(keyword)
            .and_then(|value| value.as_array().cloned())
        {
            let mut merged = Map::new();
            for member in members {
                merge_schema_objects(
                    &mut merged,
                    resolve_schema(spec, &member, depth + 1, seen),
                    true,
                );
            }
            merge_schema_objects(&mut merged, Value::Object(object.clone()), true);
            resolved = Value::Object(merged);
            break;
        }
    }
    resolved
}

fn resolve_ref(spec: &Value, value: &Value, depth: usize, seen: &mut HashSet<String>) -> Value {
    if depth >= 32 {
        return json!({});
    }
    let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
        return value.clone();
    };
    let Some(pointer) = reference.strip_prefix('#') else {
        return value.clone();
    };
    if !seen.insert(reference.to_string()) {
        return json!({});
    }
    let Some(target) = spec.pointer(pointer) else {
        seen.remove(reference);
        return value.clone();
    };
    let mut resolved = resolve_ref(spec, target, depth + 1, seen);
    seen.remove(reference);
    if let (Some(target), Some(siblings)) = (resolved.as_object_mut(), value.as_object()) {
        for (name, sibling) in siblings {
            if name != "$ref" {
                target.insert(name.clone(), sibling.clone());
            }
        }
    }
    resolved
}

fn merge_schema_objects(target: &mut Map<String, Value>, source: Value, union: bool) {
    let Some(mut source) = source.as_object().cloned() else {
        return;
    };
    if let Some(Value::Object(properties)) = source.remove("properties") {
        let target_properties = target
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("properties initialized as object");
        for (name, mut schema) in properties {
            if union && let Some(schema) = schema.as_object_mut() {
                schema.insert("nullable".to_string(), Value::Bool(true));
            }
            target_properties.entry(name).or_insert(schema);
        }
        target.insert("type".to_string(), Value::String("object".to_string()));
    }
    if !union {
        let mut required = target
            .remove("required")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        required.extend(
            source
                .remove("required")
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default(),
        );
        let unique = required
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(Value::String)
            .collect();
        target.insert("required".to_string(), Value::Array(unique));
    }
    for (name, value) in source {
        target.entry(name).or_insert(value);
    }
}

fn table_name_from_path(path: &str, method: &str) -> String {
    let mut name = sanitize_identifier(path.trim_matches('/'));
    if name.is_empty() {
        name = "root".to_string();
    }
    if method == "POST" {
        name.push_str("_post");
    }
    name
}

fn unique_identifier(base: String, fallback: &str, seen: &mut HashMap<String, usize>) -> String {
    let mut base = if base.is_empty() {
        fallback.to_string()
    } else {
        base
    };
    base.truncate(63);
    let occurrence = seen.entry(base.clone()).or_insert(0);
    *occurrence += 1;
    if *occurrence == 1 {
        return base;
    }

    let suffix = format!("_{}", *occurrence);
    let prefix_len = 63usize.saturating_sub(suffix.len());
    format!("{}{}", &base[..base.len().min(prefix_len)], suffix)
}

pub(crate) fn sanitize_identifier(raw: &str) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut result = String::new();
    for (index, character) in chars.iter().copied().enumerate() {
        if character.is_ascii_uppercase() {
            let previous = index.checked_sub(1).and_then(|index| chars.get(index));
            let next = chars.get(index + 1);
            if previous.is_some_and(|value| value.is_ascii_lowercase() || value.is_ascii_digit())
                || (previous.is_some_and(|value| value.is_ascii_uppercase())
                    && next.is_some_and(|value| value.is_ascii_lowercase()))
            {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
        } else if character.is_ascii_alphanumeric() || character == '_' {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('_') {
            result.push('_');
        }
    }
    let mut result = result.trim_matches('_').to_string();
    if result.starts_with(|character: char| character.is_ascii_digit()) {
        result.insert(0, '_');
    }
    if result.len() > 63 {
        result.truncate(63);
    }
    result
}

fn quote_ident(raw: &str) -> String {
    format!("\"{}\"", raw.replace('"', "\"\""))
}

fn quote_literal(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_enveloped_openapi_31_schema() {
        let spec = json!({
            "openapi": "3.1.0",
            "info": {"title": "Example", "version": "1"},
            "servers": [{"url": "https://example.test/api"}],
            "paths": {
                "/items": {
                    "get": {
                        "operationId": "listItems",
                        "responses": {
                            "200": {"content": {"application/json": {"schema": {
                                "type": "object",
                                "properties": {
                                    "results": {"type": "array", "items": {"$ref": "#/components/schemas/Item"}}
                                }
                            }}}}
                        }
                    }
                }
            },
            "components": {"schemas": {"Item": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer", "format": "int64"},
                    "displayName": {"type": ["string", "null"]}
                }
            }}}
        });
        let endpoints = endpoints(&spec, &HashSet::from(["GET".to_string()])).unwrap();
        assert_eq!(endpoints[0].table_name, "list_items");
        assert_eq!(endpoints[0].response_path.as_deref(), Some("/results"));
        assert_eq!(endpoints[0].columns[0].name, "id");
        assert_eq!(endpoints[0].columns[1].name, "display_name");
        assert_eq!(endpoints[0].columns[1].json_name, "displayName");
    }

    #[test]
    fn imports_geojson_properties_instead_of_collection_envelope() {
        let spec = json!({
            "openapi": "3.0.3",
            "info": {"title": "Weather", "version": "1"},
            "paths": {"/stations": {"get": {"responses": {"200": {
                "content": {"application/geo+json": {"schema": {
                    "type": "object",
                    "properties": {"features": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "geometry": {"type": "object"},
                            "properties": {"type": "object", "properties": {
                                "stationIdentifier": {"type": "string"},
                                "name": {"type": "string"}
                            }}
                        }
                    }}}
                }}}
            }}}}}
        });
        let endpoints = endpoints(&spec, &HashSet::from(["GET".to_string()])).unwrap();
        assert_eq!(endpoints[0].response_path.as_deref(), Some("/features"));
        assert_eq!(endpoints[0].object_path.as_deref(), Some("/properties"));
        assert_eq!(endpoints[0].columns[0].name, "name");
        assert_eq!(endpoints[0].columns[1].name, "station_identifier");
    }

    #[test]
    fn infers_object_type_when_openapi_omits_type() {
        let schema = json!({"properties": {"id": {"type": "integer"}}});
        assert_eq!(schema_type(&schema), Some("object"));
        assert_eq!(postgres_type(&schema), "jsonb");
    }

    #[test]
    fn imports_primitive_arrays_as_native_postgres_arrays() {
        assert_eq!(
            postgres_type(&json!({"type": "array", "items": {"type": "string"}})),
            "text[]"
        );
        assert_eq!(
            postgres_type(&json!({
                "type": "array",
                "items": {"type": "object", "properties": {"id": {"type": "integer"}}}
            })),
            "jsonb"
        );
    }

    #[test]
    fn generated_sql_quotes_every_identifier_and_keeps_attrs() {
        let endpoint = ImportedEndpoint {
            table_name: "order".to_string(),
            endpoint: "/odd'path".to_string(),
            method: "GET".to_string(),
            response_path: None,
            object_path: None,
            columns: vec![ImportedColumn {
                name: "display_name".to_string(),
                json_name: "displayName".to_string(),
                pg_type: "text",
            }],
        };
        let sql = endpoint.create_sql("target", "server", true);
        assert!(sql.contains("CREATE FOREIGN TABLE \"target\".\"order\""));
        assert!(sql.contains("endpoint '/odd''path'"));
        assert!(sql.contains("\"attrs\" jsonb"));
        assert!(sql.contains("column_map '{\"display_name\":\"displayName\"}'"));
    }

    #[test]
    fn generated_identifiers_are_nonempty_unique_and_postgres_sized() {
        let mut seen = HashMap::new();
        let first = unique_identifier(sanitize_identifier("---"), "field", &mut seen);
        let long = "x".repeat(80);
        let second = unique_identifier(long.clone(), "field", &mut seen);
        let third = unique_identifier(long, "field", &mut seen);
        assert_eq!(first, "field");
        assert_eq!(second.len(), 63);
        assert_eq!(third.len(), 63);
        assert_ne!(second, third);
    }

    #[test]
    fn catch_all_name_cannot_collide_with_api_columns() {
        let endpoint = ImportedEndpoint {
            table_name: "items".to_string(),
            endpoint: "/items".to_string(),
            method: "GET".to_string(),
            response_path: None,
            object_path: None,
            columns: vec![
                ImportedColumn {
                    name: "attrs".to_string(),
                    json_name: "attrs".to_string(),
                    pg_type: "jsonb",
                },
                ImportedColumn {
                    name: "_attrs".to_string(),
                    json_name: "_attrs".to_string(),
                    pg_type: "jsonb",
                },
            ],
        };
        let sql = endpoint.create_sql("target", "server", true);
        assert!(sql.contains("\"_attrs_2\" jsonb"));
        assert!(sql.contains("attrs_column '_attrs_2'"));
    }
}
