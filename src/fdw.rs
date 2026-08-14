use crate::error::{OpenApiFdwError, Result};
use crate::http::{
    HttpRequest, endpoint_url, execute_json, execute_mutation, fetch_spec, resolve_page_url,
};
use crate::options::{ServerConfig, TableConfig, TypeErrorMode, WriteMode};
use crate::response::{PageToken, normalize_page};
use crate::spec;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use pgrx::datum::Uuid as PgUuid;
use pgrx::pg_sys;
use pgrx::prelude::{Date, Time, Timestamp, TimestampWithTimeZone};
use pgrx::{AnyNumeric, JsonB};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use supabase_wrappers::prelude::*;
use uuid::Uuid;

#[wrappers_fdw(
    version = "0.3.2",
    author = "Sabino",
    website = "https://github.com/sabino/openapi_fdw",
    error_type = "OpenApiFdwError"
)]
pub(crate) struct OpenApiFdw {
    server: ServerConfig,
    table: Option<TableConfig>,
    columns: Vec<Column>,
    base_url: Option<url::Url>,
    initial_request: Option<HttpRequest>,
    current_url: Option<url::Url>,
    next: Option<PageToken>,
    seen_tokens: HashSet<PageToken>,
    rows: Vec<JsonValue>,
    row_index: usize,
    pages_fetched: usize,
    maximum_rows: Option<usize>,
    injected: HashMap<String, String>,
}

impl ForeignDataWrapper<OpenApiFdwError> for OpenApiFdw {
    fn new(server: ForeignServer) -> Result<Self> {
        Ok(Self {
            server: ServerConfig::from_options(&server.options)?,
            table: None,
            columns: Vec::new(),
            base_url: None,
            initial_request: None,
            current_url: None,
            next: None,
            seen_tokens: HashSet::new(),
            rows: Vec::new(),
            row_index: 0,
            pages_fetched: 0,
            maximum_rows: None,
            injected: HashMap::new(),
        })
    }

    fn get_rel_size(
        &mut self,
        quals: &[Qual],
        columns: &[Column],
        sorts: &[Sort],
        limit: &Option<Limit>,
        _options: &HashMap<String, String>,
    ) -> Result<(i64, i32)> {
        let rows = if quals.is_empty() && sorts.is_empty() {
            limit
                .as_ref()
                .map(|limit| limit.count.saturating_add(limit.offset).max(1))
                .unwrap_or(1_000)
        } else {
            1_000
        };
        let width = (columns.len().max(1) * 64).min(i32::MAX as usize) as i32;
        Ok((rows, width))
    }

    fn begin_scan(
        &mut self,
        quals: &[Qual],
        columns: &[Column],
        sorts: &[Sort],
        limit: &Option<Limit>,
        options: &HashMap<String, String>,
    ) -> Result<()> {
        let table = TableConfig::from_options(options)?;
        let Some((endpoint, used_path_quals, injected)) =
            substitute_path_parameters(&table.endpoint, quals)?
        else {
            // A path endpoint such as `/cep/{cep}` is a parameterized lookup,
            // not an enumerable collection. Catalog and BI clients commonly
            // issue an unfiltered probe while discovering fields. Treat that
            // probe as an empty relation: it preserves the declared tuple
            // descriptor, performs no outbound request, and lets a later
            // equality-constrained query execute the real lookup.
            self.table = Some(table);
            self.columns = columns.to_vec();
            self.base_url = None;
            self.initial_request = None;
            self.current_url = None;
            self.next = None;
            self.seen_tokens.clear();
            self.rows.clear();
            self.row_index = 0;
            self.pages_fetched = 0;
            self.maximum_rows = Some(0);
            self.injected.clear();
            return Ok(());
        };
        let base_url = self.resolve_base_url()?;
        let url = endpoint_url(&base_url, &endpoint)?;

        let mut query = table
            .query_params
            .iter()
            .flat_map(|(name, value)| json_query_values(name, value))
            .collect::<Vec<_>>();
        for qual in quals {
            if used_path_quals.contains(&qual.field.to_ascii_lowercase()) {
                continue;
            }
            if let Some(value) = qual_value(qual) {
                let name = table
                    .query_param_map
                    .get(&qual.field)
                    .cloned()
                    .unwrap_or_else(|| qual.field.clone());
                set_query_value(&mut query, name, value);
            }
        }

        // Wrappers exposes LIMIT/OFFSET as a planning hint but PostgreSQL keeps
        // its local Limit node. Fetch count + offset from the origin and let the
        // local node apply OFFSET exactly once.
        // A local sort or filter can discard/reorder rows after the FDW scan.
        // Bounding that scan would make LIMIT change the answer, so only use
        // it for an otherwise unconstrained, unsorted scan.
        let maximum_rows = if quals.is_empty() && sorts.is_empty() {
            limit.as_ref().and_then(|limit| {
                usize::try_from(limit.count.saturating_add(limit.offset).max(0)).ok()
            })
        } else {
            None
        };
        if let (Some(maximum), Some(name)) = (maximum_rows, table.limit_param.clone()) {
            set_query_value(&mut query, name, maximum.to_string());
        } else if let Some(page_size) = table.page_size
            && let Some(name) = table
                .page_size_param
                .clone()
                .or_else(|| table.limit_param.clone())
        {
            set_query_value(&mut query, name, page_size.to_string());
        }

        let request = HttpRequest {
            method: table.method.clone(),
            url,
            query,
            body: table.request_body.clone(),
        };

        self.table = Some(table);
        self.columns = columns.to_vec();
        self.base_url = Some(base_url);
        self.initial_request = Some(request);
        self.current_url = None;
        self.next = None;
        self.seen_tokens.clear();
        self.rows.clear();
        self.row_index = 0;
        self.pages_fetched = 0;
        self.maximum_rows = maximum_rows;
        self.injected = injected;
        self.fetch_next_page()?;
        Ok(())
    }

    fn iter_scan(&mut self, row: &mut Row) -> Result<Option<()>> {
        loop {
            if self
                .maximum_rows
                .is_some_and(|maximum| self.row_index >= maximum)
            {
                return Ok(None);
            }
            if let Some(source) = self.rows.get(self.row_index).cloned() {
                self.row_index += 1;
                self.project_row(&source, row)?;
                return Ok(Some(()));
            }
            if self.next.is_none() {
                return Ok(None);
            }
            self.fetch_next_page()?;
        }
    }

    fn re_scan(&mut self) -> Result<()> {
        self.row_index = 0;
        Ok(())
    }

    fn end_scan(&mut self) -> Result<()> {
        self.rows.clear();
        self.columns.clear();
        self.table = None;
        self.initial_request = None;
        self.current_url = None;
        self.next = None;
        self.injected.clear();
        Ok(())
    }

    fn begin_modify(&mut self, options: &HashMap<String, String>) -> Result<()> {
        let table = TableConfig::from_options(options)?;
        if table.insert.is_none() && table.update.is_none() && table.delete.is_none() {
            return Err(OpenApiFdwError::Configuration(
                "foreign table is read-only; configure an explicit mutation endpoint".to_string(),
            ));
        }
        self.base_url = Some(self.resolve_base_url()?);
        self.table = Some(table);
        Ok(())
    }

    fn insert(&mut self, row: &Row) -> Result<()> {
        let table = self.modify_table()?;
        let operation = table.insert.as_ref().ok_or_else(|| {
            OpenApiFdwError::Configuration(
                "INSERT is disabled; configure table option `insert_endpoint`".to_string(),
            )
        })?;
        let endpoint = if operation.endpoint.contains('{') {
            let rowid_column = table
                .rowid_column
                .as_deref()
                .expect("validated rowid column");
            let rowid = row_cell(row, rowid_column).ok_or_else(|| {
                OpenApiFdwError::Configuration(format!(
                    "INSERT endpoint requires a non-NULL `{rowid_column}` value"
                ))
            })?;
            mutation_endpoint(&operation.endpoint, table, rowid)?
        } else {
            operation.endpoint.clone()
        };
        let request =
            self.mutation_request(&endpoint, &operation.method, Some(write_body(row, table)?))?;
        execute_mutation(&self.server, &request)
    }

    fn update(&mut self, rowid: &Cell, new_row: &Row) -> Result<()> {
        let table = self.modify_table()?;
        let operation = table.update.as_ref().ok_or_else(|| {
            OpenApiFdwError::Configuration(
                "UPDATE is disabled; configure table option `update_endpoint`".to_string(),
            )
        })?;
        let endpoint = mutation_endpoint(&operation.endpoint, table, rowid)?;
        let request = self.mutation_request(
            &endpoint,
            &operation.method,
            Some(write_body(new_row, table)?),
        )?;
        execute_mutation(&self.server, &request)
    }

    fn delete(&mut self, rowid: &Cell) -> Result<()> {
        let table = self.modify_table()?;
        let operation = table.delete.as_ref().ok_or_else(|| {
            OpenApiFdwError::Configuration(
                "DELETE is disabled; configure table option `delete_endpoint`".to_string(),
            )
        })?;
        let endpoint = mutation_endpoint(&operation.endpoint, table, rowid)?;
        let request = self.mutation_request(&endpoint, &operation.method, None)?;
        execute_mutation(&self.server, &request)
    }

    fn end_modify(&mut self) -> Result<()> {
        self.table = None;
        self.base_url = None;
        Ok(())
    }

    fn import_foreign_schema(&mut self, stmt: ImportForeignSchemaStmt) -> Result<Vec<String>> {
        if let Some(name) = stmt
            .options
            .keys()
            .find(|name| !matches!(name.as_str(), "methods" | "include_attrs"))
        {
            return Err(OpenApiFdwError::Spec(format!(
                "unknown IMPORT option `{name}`"
            )));
        }
        let document = fetch_spec(&self.server)?;
        let methods = stmt
            .options
            .get("methods")
            .map(|methods| {
                methods
                    .split(',')
                    .map(|method| method.trim().to_ascii_uppercase())
                    .filter(|method| !method.is_empty())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_else(|| HashSet::from(["GET".to_string()]));
        if methods
            .iter()
            .any(|method| method != "GET" && method != "POST")
        {
            return Err(OpenApiFdwError::Spec(
                "IMPORT option `methods` supports only GET and POST".to_string(),
            ));
        }
        let include_attrs = import_bool(&stmt.options, "include_attrs", true)?;
        let endpoints = spec::endpoints(&document, &methods)?;
        let filtered = endpoints
            .into_iter()
            .filter(|endpoint| match stmt.list_type {
                ImportSchemaType::FdwImportSchemaAll => true,
                ImportSchemaType::FdwImportSchemaLimitTo => {
                    stmt.table_list.contains(&endpoint.table_name)
                }
                ImportSchemaType::FdwImportSchemaExcept => {
                    !stmt.table_list.contains(&endpoint.table_name)
                }
            });
        Ok(filtered
            .map(|endpoint| {
                endpoint.create_sql(&stmt.local_schema, &stmt.server_name, include_attrs)
            })
            .collect())
    }

    fn validator(options: Vec<Option<String>>, catalog: Option<pg_sys::Oid>) -> Result<()> {
        let options = validator_options(options);
        match catalog {
            Some(oid) if oid == FOREIGN_SERVER_RELATION_ID => {
                reject_unknown(&options, SERVER_OPTIONS)?;
                ServerConfig::from_options(&options).map(|_| ())
            }
            Some(oid) if oid == FOREIGN_TABLE_RELATION_ID => {
                reject_unknown(&options, TABLE_OPTIONS)?;
                TableConfig::from_options(&options).map(|_| ())
            }
            _ => Ok(()),
        }
    }
}

impl OpenApiFdw {
    fn modify_table(&self) -> Result<&TableConfig> {
        self.table.as_ref().ok_or_else(|| {
            OpenApiFdwError::Configuration(
                "foreign table modification was not initialized".to_string(),
            )
        })
    }

    fn mutation_request(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<JsonValue>,
    ) -> Result<HttpRequest> {
        let base_url = self.base_url.as_ref().ok_or_else(|| {
            OpenApiFdwError::Configuration(
                "foreign table modification has no resolved base URL".to_string(),
            )
        })?;
        Ok(HttpRequest {
            method: method.to_string(),
            url: endpoint_url(base_url, endpoint)?,
            query: Vec::new(),
            body,
        })
    }

    fn resolve_base_url(&self) -> Result<url::Url> {
        if let Some(url) = &self.server.base_url {
            return Ok(url.clone());
        }
        let document = fetch_spec(&self.server)?;
        spec::base_url_from_spec(
            &document,
            self.server.spec_url.as_ref(),
            self.server.allow_http,
        )
    }

    fn fetch_next_page(&mut self) -> Result<()> {
        let table = self.table.as_ref().expect("scan table configuration");
        let page_limit = table.max_pages.unwrap_or(self.server.max_pages);
        if self.pages_fetched >= page_limit {
            return Err(OpenApiFdwError::Response(format!(
                "pagination exceeded max_pages {page_limit}"
            )));
        }

        let mut request = self.initial_request.as_ref().expect("scan request").clone();
        if let Some(token) = self.next.take() {
            match token {
                PageToken::Url(raw) => {
                    request.url = resolve_page_url(
                        self.base_url.as_ref().expect("scan base URL"),
                        self.current_url.as_ref().unwrap_or(&request.url),
                        &raw,
                        self.server.allow_cross_origin_pagination,
                    )?;
                    request.query.clear();
                }
                PageToken::Cursor(cursor) => {
                    let name = table.cursor_param.clone().ok_or_else(|| {
                        OpenApiFdwError::Configuration(
                            "cursor pagination requires table option `cursor_param`".to_string(),
                        )
                    })?;
                    set_query_value(&mut request.query, name, cursor);
                }
            }
        }

        let response = execute_json(&self.server, &request)?;
        self.current_url = Some(response.effective_url);
        let page = normalize_page(response.value, &response.link_headers, table)?;
        self.pages_fetched += 1;
        if let Some(next) = page.next {
            if !self.seen_tokens.insert(next.clone()) {
                return Err(OpenApiFdwError::Response(
                    "pagination returned a duplicate cursor or URL".to_string(),
                ));
            }
            self.next = Some(next);
        }
        self.rows.extend(page.rows);
        Ok(())
    }

    fn project_row(&self, source: &JsonValue, output: &mut Row) -> Result<()> {
        let table = self.table.as_ref().expect("scan table configuration");
        let projected = match table.object_path.as_deref() {
            Some(pointer) => source.pointer(pointer).ok_or_else(|| {
                OpenApiFdwError::Response(format!(
                    "row does not contain configured object_path `{pointer}`"
                ))
            })?,
            None => source,
        };

        for column in &self.columns {
            if column.name == table.attrs_column {
                output.push(&column.name, Some(Cell::Json(JsonB(source.clone()))));
                continue;
            }

            let injected = self
                .injected
                .get(&column.name)
                .map(|value| JsonValue::String(value.clone()));
            let value = self
                .column_value(projected, &column.name, table)
                .or(injected.as_ref());
            let cell = match value {
                None | Some(JsonValue::Null) => None,
                Some(value) => match json_to_cell(value, column) {
                    Ok(cell) => Some(cell),
                    Err(_) if table.type_error == TypeErrorMode::Null => None,
                    Err(error) => return Err(error),
                },
            };
            output.push(&column.name, cell);
        }
        Ok(())
    }

    fn column_value<'a>(
        &'a self,
        projected: &'a JsonValue,
        column: &str,
        table: &TableConfig,
    ) -> Option<&'a JsonValue> {
        let mapped = table.column_map.get(column).map(String::as_str);
        let found = match mapped {
            Some(pointer) if pointer.starts_with('/') => projected.pointer(pointer),
            Some(name) => projected.get(name),
            None => projected.get(column).or_else(|| {
                projected.as_object().and_then(|object| {
                    object
                        .iter()
                        .find(|(name, _)| spec::sanitize_identifier(name) == column)
                        .map(|(_, value)| value)
                })
            }),
        };
        found.or_else(|| {
            if projected.is_object() {
                None
            } else if column == "value" {
                Some(projected)
            } else {
                None
            }
        })
    }
}

fn row_cell<'a>(row: &'a Row, column: &str) -> Option<&'a Cell> {
    row.iter()
        .find(|(name, _)| name.as_str() == column)
        .and_then(|(_, cell)| cell.as_ref())
}

fn mutation_endpoint(template: &str, table: &TableConfig, rowid: &Cell) -> Result<String> {
    let parameter = table
        .rowid_parameter
        .as_deref()
        .expect("mutation endpoint has validated rowid parameter");
    let raw = cell_path_value(rowid)?;
    let encoded = utf8_percent_encode(&raw, NON_ALPHANUMERIC).to_string();
    Ok(template.replace(&format!("{{{parameter}}}"), &encoded))
}

fn write_body(row: &Row, table: &TableConfig) -> Result<JsonValue> {
    let mut body = match table.write_mode {
        WriteMode::Columns => serde_json::Map::new(),
        WriteMode::Attrs | WriteMode::Merge => {
            let attrs = row_cell(row, &table.attrs_column);
            match attrs {
                Some(Cell::Json(JsonB(JsonValue::Object(object)))) => object.clone(),
                Some(Cell::Json(_)) => {
                    return Err(OpenApiFdwError::Configuration(format!(
                        "write_mode `{}` requires `{}` to contain a JSON object",
                        write_mode_name(table.write_mode),
                        table.attrs_column
                    )));
                }
                Some(_) => {
                    return Err(OpenApiFdwError::Configuration(format!(
                        "attrs_column `{}` must use PostgreSQL type jsonb",
                        table.attrs_column
                    )));
                }
                None if table.write_mode == WriteMode::Attrs => {
                    return Err(OpenApiFdwError::Configuration(format!(
                        "write_mode `attrs` requires a non-NULL `{}` JSONB value",
                        table.attrs_column
                    )));
                }
                None => serde_json::Map::new(),
            }
        }
    };

    if table.write_mode != WriteMode::Attrs {
        for (column, cell) in row.iter() {
            if table.rowid_column.as_deref() == Some(column.as_str())
                || column == &table.attrs_column
            {
                continue;
            }
            if let Some(columns) = &table.write_columns
                && !columns.contains(column)
            {
                continue;
            }
            let value = match cell {
                Some(cell) => cell_to_json(cell)?,
                None => JsonValue::Null,
            };
            let target = table
                .column_map
                .get(column)
                .map(String::as_str)
                .unwrap_or(column);
            insert_write_value(&mut body, target, value)?;
        }
    }

    Ok(JsonValue::Object(body))
}

fn write_mode_name(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Columns => "columns",
        WriteMode::Attrs => "attrs",
        WriteMode::Merge => "merge",
    }
}

fn insert_write_value(
    body: &mut serde_json::Map<String, JsonValue>,
    target: &str,
    value: JsonValue,
) -> Result<()> {
    if !target.starts_with('/') {
        body.insert(target.to_string(), value);
        return Ok(());
    }
    let tokens = target
        .split('/')
        .skip(1)
        .map(decode_pointer_token)
        .collect::<Result<Vec<_>>>()?;
    if tokens.is_empty() {
        return Err(OpenApiFdwError::Configuration(
            "column_map JSON Pointer cannot target the document root during writes".to_string(),
        ));
    }
    insert_pointer_tokens(body, &tokens, value, target)
}

fn insert_pointer_tokens(
    object: &mut serde_json::Map<String, JsonValue>,
    tokens: &[String],
    value: JsonValue,
    pointer: &str,
) -> Result<()> {
    let (head, tail) = tokens.split_first().expect("non-empty pointer tokens");
    if tail.is_empty() {
        object.insert(head.clone(), value);
        return Ok(());
    }
    let nested = object
        .entry(head.clone())
        .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
    let nested = nested.as_object_mut().ok_or_else(|| {
        OpenApiFdwError::Configuration(format!(
            "column_map pointer `{pointer}` conflicts with a non-object JSON value"
        ))
    })?;
    insert_pointer_tokens(nested, tail, value, pointer)
}

fn decode_pointer_token(raw: &str) -> Result<String> {
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            _ => {
                return Err(OpenApiFdwError::Configuration(
                    "column_map contains an invalid RFC 6901 escape".to_string(),
                ));
            }
        }
    }
    Ok(decoded)
}

fn cell_path_value(cell: &Cell) -> Result<String> {
    match cell_to_json(cell)? {
        JsonValue::Bool(value) => Ok(value.to_string()),
        JsonValue::Number(value) => Ok(value.to_string()),
        JsonValue::String(value) => Ok(value),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(OpenApiFdwError::Configuration(
                "rowid_column must use a scalar PostgreSQL type".to_string(),
            ))
        }
    }
}

fn cell_to_json(cell: &Cell) -> Result<JsonValue> {
    let number = |value: f64| {
        serde_json::Number::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| {
                OpenApiFdwError::Configuration(
                    "NaN and infinite floating-point values cannot be sent as JSON".to_string(),
                )
            })
    };
    let string_array = |values: &[Option<String>]| {
        JsonValue::Array(
            values
                .iter()
                .map(|value| {
                    value
                        .clone()
                        .map(JsonValue::String)
                        .unwrap_or(JsonValue::Null)
                })
                .collect(),
        )
    };
    match cell {
        Cell::Bool(value) => Ok(JsonValue::Bool(*value)),
        Cell::I8(value) => Ok(JsonValue::from(*value)),
        Cell::I16(value) => Ok(JsonValue::from(*value)),
        Cell::I32(value) => Ok(JsonValue::from(*value)),
        Cell::I64(value) => Ok(JsonValue::from(*value)),
        Cell::F32(value) => number(*value as f64),
        Cell::F64(value) => number(*value),
        Cell::Numeric(value) => serde_json::from_str(&value.to_string()).map_err(|_| {
            OpenApiFdwError::Configuration(
                "numeric value cannot be represented as a JSON number".to_string(),
            )
        }),
        Cell::String(value) => Ok(JsonValue::String(value.clone())),
        Cell::Date(value) => Ok(JsonValue::String(pg_display_value(value))),
        Cell::Time(value) => Ok(JsonValue::String(pg_display_value(value))),
        Cell::Timestamp(value) => Ok(JsonValue::String(pg_display_value(value))),
        Cell::Timestamptz(value) => Ok(JsonValue::String(pg_display_value(value))),
        Cell::Interval(value) => Ok(JsonValue::String(value.to_string())),
        Cell::Json(JsonB(value)) => Ok(value.clone()),
        Cell::Bytea(value) => {
            let bytes = unsafe { pgrx::varlena::varlena_to_byte_slice(*value) };
            let mut encoded = String::with_capacity(2 + bytes.len() * 2);
            encoded.push_str("\\x");
            for byte in bytes {
                write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
            }
            Ok(JsonValue::String(encoded))
        }
        Cell::Uuid(value) => Ok(JsonValue::String(value.to_string())),
        Cell::BoolArray(values) => Ok(JsonValue::Array(
            values
                .iter()
                .map(|value| value.map(JsonValue::Bool).unwrap_or(JsonValue::Null))
                .collect(),
        )),
        Cell::I16Array(values) => Ok(integer_array(values)),
        Cell::I32Array(values) => Ok(integer_array(values)),
        Cell::I64Array(values) => Ok(integer_array(values)),
        Cell::F32Array(values) => values
            .iter()
            .map(|value| value.map(|value| number(value as f64)).transpose())
            .collect::<Result<Vec<_>>>()
            .map(|values| {
                JsonValue::Array(
                    values
                        .into_iter()
                        .map(|value| value.unwrap_or(JsonValue::Null))
                        .collect(),
                )
            }),
        Cell::F64Array(values) => values
            .iter()
            .map(|value| value.map(number).transpose())
            .collect::<Result<Vec<_>>>()
            .map(|values| {
                JsonValue::Array(
                    values
                        .into_iter()
                        .map(|value| value.unwrap_or(JsonValue::Null))
                        .collect(),
                )
            }),
        Cell::StringArray(values) => Ok(string_array(values)),
    }
}

fn integer_array<T>(values: &[Option<T>]) -> JsonValue
where
    T: Copy + Into<JsonValue>,
{
    JsonValue::Array(
        values
            .iter()
            .map(|value| value.map(Into::into).unwrap_or(JsonValue::Null))
            .collect(),
    )
}

fn pg_display_value(value: &impl std::fmt::Display) -> String {
    value.to_string().trim_matches('\'').to_string()
}

fn substitute_path_parameters(
    template: &str,
    quals: &[Qual],
) -> Result<Option<(String, HashSet<String>, HashMap<String, String>)>> {
    let mut values = HashMap::new();
    for qual in quals {
        if let Some(value) = qual_value(qual) {
            values.insert(qual.field.to_ascii_lowercase(), value);
        }
    }

    let mut endpoint = String::new();
    let mut remaining = template;
    let mut used = HashSet::new();
    let mut injected = HashMap::new();
    let mut missing = false;
    while let Some(start) = remaining.find('{') {
        endpoint.push_str(&remaining[..start]);
        let after_open = &remaining[start + 1..];
        let end = after_open.find('}').ok_or_else(|| {
            OpenApiFdwError::Configuration("endpoint has an unmatched `{`".to_string())
        })?;
        let parameter = &after_open[..end];
        let field = spec::sanitize_identifier(parameter);
        if let Some(value) = values.get(&field.to_ascii_lowercase()) {
            endpoint.push_str(&utf8_percent_encode(value, NON_ALPHANUMERIC).to_string());
            used.insert(field.to_ascii_lowercase());
            injected.insert(field, value.clone());
        } else {
            missing = true;
        }
        remaining = &after_open[end + 1..];
    }
    endpoint.push_str(remaining);
    Ok((!missing).then_some((endpoint, used, injected)))
}

fn qual_value(qual: &Qual) -> Option<String> {
    if qual.operator != "=" || qual.use_or {
        return None;
    }
    let Value::Cell(cell) = &qual.value else {
        return None;
    };
    match cell {
        Cell::Bool(value) => Some(value.to_string()),
        Cell::I8(value) => Some(value.to_string()),
        Cell::I16(value) => Some(value.to_string()),
        Cell::I32(value) => Some(value.to_string()),
        Cell::I64(value) => Some(value.to_string()),
        Cell::F32(value) => Some(value.to_string()),
        Cell::F64(value) => Some(value.to_string()),
        Cell::Numeric(value) => Some(value.to_string()),
        Cell::String(value) => Some(value.clone()),
        Cell::Uuid(value) => Some(value.to_string()),
        Cell::Date(value) => Some(value.to_string().trim_matches('\'').to_string()),
        Cell::Time(value) => Some(value.to_string().trim_matches('\'').to_string()),
        Cell::Timestamp(value) => Some(value.to_string().trim_matches('\'').to_string()),
        Cell::Timestamptz(value) => Some(value.to_string().trim_matches('\'').to_string()),
        _ => None,
    }
}

fn json_query_values(name: &str, value: &JsonValue) -> Vec<(String, String)> {
    match value {
        JsonValue::Array(values) => values
            .iter()
            .filter_map(json_scalar_string)
            .map(|value| (name.to_string(), value))
            .collect(),
        _ => json_scalar_string(value)
            .map(|value| vec![(name.to_string(), value)])
            .unwrap_or_else(|| vec![(name.to_string(), value.to_string())]),
    }
}

fn json_scalar_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Null => None,
        JsonValue::Bool(value) => Some(value.to_string()),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn set_query_value(query: &mut Vec<(String, String)>, name: String, value: String) {
    query.retain(|(existing, _)| existing != &name);
    query.push((name, value));
}

fn json_to_cell(value: &JsonValue, column: &Column) -> Result<Cell> {
    let conversion = |target: &'static str, reason: String| OpenApiFdwError::Conversion {
        column: column.name.clone(),
        target,
        reason,
    };
    match column.type_oid {
        pg_sys::BOOLOID => parse_bool(value)
            .map(Cell::Bool)
            .ok_or_else(|| conversion("boolean", "expected true or false".to_string())),
        pg_sys::CHAROID => parse_i64(value)
            .and_then(|value| i8::try_from(value).ok())
            .map(Cell::I8)
            .ok_or_else(|| conversion("\"char\"", "value is outside int8 range".to_string())),
        pg_sys::INT2OID => parse_i64(value)
            .and_then(|value| i16::try_from(value).ok())
            .map(Cell::I16)
            .ok_or_else(|| conversion("smallint", "value is outside int16 range".to_string())),
        pg_sys::INT4OID => parse_i64(value)
            .and_then(|value| i32::try_from(value).ok())
            .map(Cell::I32)
            .ok_or_else(|| conversion("integer", "value is outside int32 range".to_string())),
        pg_sys::INT8OID => parse_i64(value)
            .map(Cell::I64)
            .ok_or_else(|| conversion("bigint", "expected an integer".to_string())),
        pg_sys::FLOAT4OID => parse_f64(value)
            .map(|value| Cell::F32(value as f32))
            .ok_or_else(|| conversion("real", "expected a finite number".to_string())),
        pg_sys::FLOAT8OID => parse_f64(value)
            .map(Cell::F64)
            .ok_or_else(|| conversion("double precision", "expected a finite number".to_string())),
        pg_sys::NUMERICOID => value_text(value)
            .and_then(|value| AnyNumeric::try_from(value.as_str()).ok())
            .map(Cell::Numeric)
            .ok_or_else(|| conversion("numeric", "expected a decimal number".to_string())),
        pg_sys::TEXTOID | pg_sys::VARCHAROID | pg_sys::BPCHAROID | pg_sys::NAMEOID => {
            Ok(Cell::String(value_text(value).unwrap_or_default()))
        }
        pg_sys::JSONOID | pg_sys::JSONBOID => Ok(Cell::Json(JsonB(value.clone()))),
        pg_sys::UUIDOID => value
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .map(|value| Cell::Uuid(PgUuid::from_bytes(*value.as_bytes())))
            .ok_or_else(|| conversion("uuid", "expected a UUID string".to_string())),
        pg_sys::DATEOID => parse_pg::<Date>(value, "date", &conversion).map(Cell::Date),
        pg_sys::TIMEOID => parse_pg::<Time>(value, "time", &conversion).map(Cell::Time),
        pg_sys::TIMESTAMPOID => {
            parse_pg::<Timestamp>(value, "timestamp", &conversion).map(Cell::Timestamp)
        }
        pg_sys::TIMESTAMPTZOID => {
            parse_pg::<TimestampWithTimeZone>(value, "timestamp with time zone", &conversion)
                .map(Cell::Timestamptz)
        }
        pg_sys::BOOLARRAYOID => {
            parse_array(value, parse_bool, "boolean[]", &conversion).map(Cell::BoolArray)
        }
        pg_sys::INT2ARRAYOID => parse_array(
            value,
            |value| parse_i64(value).and_then(|value| i16::try_from(value).ok()),
            "smallint[]",
            &conversion,
        )
        .map(Cell::I16Array),
        pg_sys::INT4ARRAYOID => parse_array(
            value,
            |value| parse_i64(value).and_then(|value| i32::try_from(value).ok()),
            "integer[]",
            &conversion,
        )
        .map(Cell::I32Array),
        pg_sys::INT8ARRAYOID => {
            parse_array(value, parse_i64, "bigint[]", &conversion).map(Cell::I64Array)
        }
        pg_sys::FLOAT4ARRAYOID => parse_array(
            value,
            |value| parse_f64(value).map(|value| value as f32),
            "real[]",
            &conversion,
        )
        .map(Cell::F32Array),
        pg_sys::FLOAT8ARRAYOID => {
            parse_array(value, parse_f64, "double precision[]", &conversion).map(Cell::F64Array)
        }
        pg_sys::TEXTARRAYOID | pg_sys::VARCHARARRAYOID | pg_sys::BPCHARARRAYOID => parse_array(
            value,
            |value| Some(value_text(value).unwrap_or_default()),
            "text[]",
            &conversion,
        )
        .map(Cell::StringArray),
        oid => Err(conversion(
            "supported PostgreSQL scalar or array type",
            format!("type OID {oid} is not supported"),
        )),
    }
}

fn parse_bool(value: &JsonValue) -> Option<bool> {
    value.as_bool().or_else(|| match value.as_str()? {
        "true" | "t" | "1" => Some(true),
        "false" | "f" | "0" => Some(false),
        _ => None,
    })
}

fn parse_i64(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn parse_f64(value: &JsonValue) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .filter(|value| value.is_finite())
}

fn value_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(value) => Some(value.clone()),
        value => Some(value.to_string()),
    }
}

fn parse_pg<T>(
    value: &JsonValue,
    target: &'static str,
    conversion: &impl Fn(&'static str, String) -> OpenApiFdwError,
) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = value
        .as_str()
        .ok_or_else(|| conversion(target, "expected a formatted string".to_string()))?;
    raw.parse::<T>()
        .map_err(|error| conversion(target, format!("invalid formatted string: {error}")))
}

fn parse_array<T>(
    value: &JsonValue,
    parse: impl Fn(&JsonValue) -> Option<T>,
    target: &'static str,
    conversion: &impl Fn(&'static str, String) -> OpenApiFdwError,
) -> Result<Vec<Option<T>>> {
    let values = value
        .as_array()
        .ok_or_else(|| conversion(target, "expected a JSON array".to_string()))?;
    values
        .iter()
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                parse(value)
                    .map(Some)
                    .ok_or_else(|| conversion(target, "array element has wrong type".to_string()))
            }
        })
        .collect()
}

fn validator_options(options: Vec<Option<String>>) -> HashMap<String, String> {
    options
        .into_iter()
        .flatten()
        .filter_map(|option| {
            option
                .split_once('=')
                .map(|(name, value)| (name.to_string(), value.to_string()))
        })
        .collect()
}

fn reject_unknown(options: &HashMap<String, String>, allowed: &[&str]) -> Result<()> {
    if let Some(name) = options
        .keys()
        .find(|name| !allowed.contains(&name.as_str()))
    {
        return Err(OpenApiFdwError::Configuration(format!(
            "unknown option `{name}`"
        )));
    }
    Ok(())
}

fn import_bool(options: &HashMap<String, String>, name: &str, default: bool) -> Result<bool> {
    match options.get(name).map(String::as_str) {
        None => Ok(default),
        Some("true" | "on" | "yes" | "1") => Ok(true),
        Some("false" | "off" | "no" | "0") => Ok(false),
        Some(_) => Err(OpenApiFdwError::Spec(format!(
            "IMPORT option `{name}` must be a boolean"
        ))),
    }
}

const SERVER_OPTIONS: &[&str] = &[
    "base_url",
    "spec_url",
    "spec_json",
    "headers",
    "headers_env",
    "user_agent",
    "accept",
    "api_key",
    "api_key_env",
    "api_key_location",
    "api_key_name",
    "api_key_prefix",
    "bearer_token",
    "bearer_token_env",
    "connect_timeout_ms",
    "request_timeout_ms",
    "max_response_bytes",
    "max_pages",
    "max_retries",
    "max_retry_delay_ms",
    "max_redirects",
    "allow_http",
    "allow_cross_origin_pagination",
    "spec_with_auth",
];

const TABLE_OPTIONS: &[&str] = &[
    "endpoint",
    "method",
    "response_path",
    "object_path",
    "query_params",
    "request_body",
    "column_map",
    "query_param_map",
    "attrs_column",
    "limit_param",
    "page_size",
    "page_size_param",
    "cursor_path",
    "cursor_param",
    "pagination",
    "max_pages",
    "on_type_error",
    "startup_cost",
    "rowid_column",
    "rowid_parameter",
    "insert_endpoint",
    "insert_method",
    "update_endpoint",
    "update_method",
    "delete_endpoint",
    "delete_method",
    "write_mode",
    "write_columns",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn string_qual(field: &str, value: &str) -> Qual {
        Qual {
            field: field.to_string(),
            operator: "=".to_string(),
            value: Value::Cell(Cell::String(value.to_string())),
            use_or: false,
            param: None,
        }
    }

    #[test]
    fn substitutes_and_encodes_path_parameters() {
        let (endpoint, used, injected) = substitute_path_parameters(
            "/pokemon/{pokemon_name}",
            &[string_qual("pokemon_name", "mr. mime")],
        )
        .unwrap()
        .unwrap();
        assert_eq!(endpoint, "/pokemon/mr%2E%20mime");
        assert!(used.contains("pokemon_name"));
        assert_eq!(injected["pokemon_name"], "mr. mime");
    }

    #[test]
    fn missing_path_parameters_leave_the_lookup_unbound() {
        assert!(
            substitute_path_parameters("/items/{id}", &[])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_path_templates_still_fail_when_unbound() {
        assert!(substitute_path_parameters("/items/{id", &[]).is_err());
    }

    #[test]
    fn static_json_array_becomes_repeated_query_parameter() {
        assert_eq!(
            json_query_values("tag", &serde_json::json!(["one", "two"])),
            vec![
                ("tag".to_string(), "one".to_string()),
                ("tag".to_string(), "two".to_string())
            ]
        );
    }

    #[test]
    fn column_write_mode_applies_whitelist_and_json_names() {
        let table = TableConfig::from_options(&HashMap::from([
            ("endpoint".to_string(), "/items".to_string()),
            ("rowid_column".to_string(), "id".to_string()),
            ("insert_endpoint".to_string(), "/items".to_string()),
            (
                "write_columns".to_string(),
                r#"["display_name","data"]"#.to_string(),
            ),
            (
                "column_map".to_string(),
                r#"{"display_name":"displayName"}"#.to_string(),
            ),
        ]))
        .unwrap();
        let mut row = Row::new();
        row.push("id", Some(Cell::String("ignored-id".to_string())));
        row.push("display_name", Some(Cell::String("Example".to_string())));
        row.push(
            "data",
            Some(Cell::Json(JsonB(serde_json::json!({"dynamic": true})))),
        );
        row.push("server_only", Some(Cell::String("ignored".to_string())));
        row.push(
            "attrs",
            Some(Cell::Json(JsonB(serde_json::json!({"raw": "ignored"})))),
        );

        assert_eq!(
            write_body(&row, &table).unwrap(),
            serde_json::json!({
                "displayName": "Example",
                "data": {"dynamic": true}
            })
        );

        let mut partial_update = Row::new();
        partial_update.push(
            "data",
            Some(Cell::Json(JsonB(serde_json::json!({"only": "changed"})))),
        );
        assert_eq!(
            write_body(&partial_update, &table).unwrap(),
            serde_json::json!({"data": {"only": "changed"}})
        );
    }

    #[test]
    fn attrs_write_mode_forwards_a_dynamic_json_object() {
        let table = TableConfig::from_options(&HashMap::from([
            ("endpoint".to_string(), "/items".to_string()),
            ("rowid_column".to_string(), "id".to_string()),
            ("insert_endpoint".to_string(), "/items".to_string()),
            ("write_mode".to_string(), "attrs".to_string()),
        ]))
        .unwrap();
        let mut row = Row::new();
        row.push(
            "attrs",
            Some(Cell::Json(JsonB(serde_json::json!({
                "unknownFutureField": 42
            })))),
        );
        assert_eq!(
            write_body(&row, &table).unwrap(),
            serde_json::json!({"unknownFutureField": 42})
        );
    }

    #[test]
    fn mutation_identity_is_url_encoded() {
        let table = TableConfig::from_options(&HashMap::from([
            ("endpoint".to_string(), "/items".to_string()),
            ("rowid_column".to_string(), "id".to_string()),
            ("rowid_parameter".to_string(), "itemId".to_string()),
            ("delete_endpoint".to_string(), "/items/{itemId}".to_string()),
        ]))
        .unwrap();
        assert_eq!(
            mutation_endpoint(
                &table.delete.as_ref().unwrap().endpoint,
                &table,
                &Cell::String("a/b c".to_string())
            )
            .unwrap(),
            "/items/a%2Fb%20c"
        );
    }
}
