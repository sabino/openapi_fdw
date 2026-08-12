use crate::error::{OpenApiFdwError, Result};
use crate::options::{PaginationMode, TableConfig};
use serde_json::Value;

const ENVELOPE_KEYS: &[&str] = &[
    "data", "results", "items", "records", "entries", "features", "@graph",
];
const NEXT_URL_PATHS: &[&str] = &[
    "/meta/pagination/next",
    "/meta/pagination/next_url",
    "/pagination/next",
    "/pagination/next_url",
    "/links/next",
    "/links/next_url",
    "/next",
    "/next_url",
    "/_links/next/href",
];
const HAS_MORE_PATHS: &[&str] = &[
    "/meta/pagination/has_more",
    "/pagination/has_more",
    "/has_more",
];
const CURSOR_PATHS: &[&str] = &[
    "/meta/pagination/next_cursor",
    "/pagination/next_cursor",
    "/next_cursor",
    "/cursor",
];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PageToken {
    Url(String),
    Cursor(String),
}

#[derive(Debug)]
pub(crate) struct Page {
    pub(crate) rows: Vec<Value>,
    pub(crate) next: Option<PageToken>,
}

pub(crate) fn normalize_page(
    body: Value,
    link_headers: &[String],
    table: &TableConfig,
) -> Result<Page> {
    let next = if table.pagination == PaginationMode::Auto {
        find_next(&body, link_headers, table)
    } else {
        None
    };

    let selected = match table.response_path.as_deref() {
        Some(pointer) => body.pointer(pointer).cloned().ok_or_else(|| {
            OpenApiFdwError::Response(format!(
                "configured response_path `{pointer}` does not exist"
            ))
        })?,
        None => auto_select(body),
    };

    let rows = match selected {
        Value::Null => Vec::new(),
        Value::Array(rows) => rows,
        value => vec![value],
    };

    // Empty pages with a next token are usually a broken pagination contract.
    // Stopping here avoids hammering an API or looping over an unbounded cursor.
    Ok(Page {
        next: if rows.is_empty() { None } else { next },
        rows,
    })
}

fn auto_select(body: Value) -> Value {
    if let Value::Object(object) = &body {
        for key in ENVELOPE_KEYS {
            if let Some(candidate) = object.get(*key)
                && (candidate.is_array() || candidate.is_object())
            {
                return candidate.clone();
            }
        }
    }
    body
}

fn find_next(body: &Value, link_headers: &[String], table: &TableConfig) -> Option<PageToken> {
    if let Some(pointer) = table.cursor_path.as_deref()
        && let Some(value) = scalar_string(body.pointer(pointer))
    {
        return Some(classify_token(value));
    }

    if let Some(url) = link_headers.iter().find_map(|value| parse_link_next(value)) {
        return Some(PageToken::Url(url));
    }

    for pointer in NEXT_URL_PATHS {
        if let Some(value) = scalar_string(body.pointer(pointer)) {
            return Some(PageToken::Url(value));
        }
    }

    let has_more = HAS_MORE_PATHS
        .iter()
        .find_map(|pointer| body.pointer(pointer))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if has_more {
        for pointer in CURSOR_PATHS {
            if let Some(value) = scalar_string(body.pointer(pointer)) {
                return Some(PageToken::Cursor(value));
            }
        }
    }
    None
}

fn scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn classify_token(value: String) -> PageToken {
    if value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with('/')
        || value.starts_with('?')
    {
        PageToken::Url(value)
    } else {
        PageToken::Cursor(value)
    }
}

pub(crate) fn parse_link_next(raw: &str) -> Option<String> {
    for entry in split_link_entries(raw) {
        let entry = entry.trim();
        let Some(without_open) = entry.strip_prefix('<') else {
            continue;
        };
        let Some(close) = without_open.find('>') else {
            continue;
        };
        let url = &entry[1..=close];
        let params = &entry[close + 2..];
        if params.split(';').any(|parameter| {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                return false;
            };
            name.trim().eq_ignore_ascii_case("rel")
                && value
                    .trim()
                    .trim_matches('"')
                    .split_ascii_whitespace()
                    .any(|relation| relation.eq_ignore_ascii_case("next"))
        }) {
            return Some(url.to_string());
        }
    }
    None
}

fn split_link_entries(raw: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut in_angle = false;
    let mut in_quotes = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, character) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if in_quotes => escaped = true,
            '<' if !in_quotes => in_angle = true,
            '>' if !in_quotes => in_angle = false,
            '"' if !in_angle => in_quotes = !in_quotes,
            ',' if !in_angle && !in_quotes => {
                entries.push(&raw[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    entries.push(&raw[start..]);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn table(extra: &[(&str, &str)]) -> TableConfig {
        let mut options = HashMap::from([("endpoint".to_string(), "/items".to_string())]);
        options.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );
        TableConfig::from_options(&options).unwrap()
    }

    #[test]
    fn unwraps_results_and_finds_next_url() {
        let body = serde_json::json!({
            "count": 2,
            "next": "https://example.test/items?offset=2",
            "results": [{"id": 1}, {"id": 2}]
        });
        let page = normalize_page(body, &[], &table(&[])).unwrap();
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            page.next,
            Some(PageToken::Url(
                "https://example.test/items?offset=2".to_string()
            ))
        );
    }

    #[test]
    fn uses_explicit_json_pointer() {
        let body = serde_json::json!({"payload": {"rows": [{"id": 7}]}});
        let page =
            normalize_page(body, &[], &table(&[("response_path", "/payload/rows")])).unwrap();
        assert_eq!(page.rows, vec![serde_json::json!({"id": 7})]);
    }

    #[test]
    fn parses_rfc_link_header() {
        let header = concat!(
            "<https://example.test/items?page=2>; rel=\"next\", ",
            "<https://example.test/items?page=9>; rel=\"last\""
        );
        assert_eq!(
            parse_link_next(header).as_deref(),
            Some("https://example.test/items?page=2")
        );
    }

    #[test]
    fn supports_geojson_features() {
        let body = serde_json::json!({
            "type": "FeatureCollection",
            "features": [{"type": "Feature", "properties": {"name": "KSEA"}}]
        });
        let page = normalize_page(body, &[], &table(&[])).unwrap();
        assert_eq!(page.rows[0]["properties"]["name"], "KSEA");
    }
}
