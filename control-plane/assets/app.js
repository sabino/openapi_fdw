"use strict";

const byId = (id) => document.getElementById(id);
const state = {
  control: null,
  discovery: null,
  sample: null,
  importBundle: null,
};

function make(tag, className, text) {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text !== undefined) element.textContent = text;
  return element;
}

async function api(path, options = {}) {
  const method = options.method || "GET";
  const headers = new Headers(options.headers || {});
  headers.set("Accept", "application/json");
  if (method !== "GET" && method !== "HEAD") {
    headers.set("X-OpenAPI-FDW-Request", "control-plane");
  }
  if (options.body !== undefined) {
    headers.set("Content-Type", "application/json");
  }
  const response = await fetch(path, {
    method,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    credentials: "same-origin",
  });
  if (response.status === 401) {
    window.location.assign("/login");
    throw new Error("Authentication required");
  }
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(payload.error || `Request failed with HTTP ${response.status}`);
  }
  return payload;
}

function toast(message, isError = false) {
  const element = byId("toast");
  element.textContent = message;
  element.classList.toggle("toast-error", isError);
  element.classList.remove("hidden");
  window.clearTimeout(toast.timeout);
  toast.timeout = window.setTimeout(() => element.classList.add("hidden"), 5200);
}

function notice(message, isError = false) {
  const element = byId("global-notice");
  element.textContent = message;
  element.className = `notice ${isError ? "notice-error" : "notice-info"}`;
}

function clearNotice() {
  byId("global-notice").classList.add("hidden");
}

function setBusy(button, busy, label) {
  if (busy) {
    button.dataset.previousLabel = button.textContent;
    button.textContent = label || "Working…";
    button.disabled = true;
  } else {
    button.textContent = button.dataset.previousLabel || button.textContent;
    button.disabled = false;
  }
}

function jsonObjectField(id, label) {
  const raw = byId(id).value.trim();
  if (!raw) return {};
  let value;
  try {
    value = JSON.parse(raw);
  } catch (error) {
    throw new Error(`${label} must be valid JSON: ${error.message}`);
  }
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    throw new Error(`${label} must be a JSON object.`);
  }
  return value;
}

function sourceFromForm(tables = []) {
  const inline = byId("spec-json").value.trim();
  const specUrl = byId("spec-url").value.trim();
  const methods = [];
  if (byId("method-get").checked) methods.push("GET");
  if (byId("method-post").checked) methods.push("POST");

  const authType = byId("auth-type").value;
  let auth = { type: "none" };
  if (authType !== "none") {
    const secretValue = byId("secret-value").value;
    const secret = byId("secret-mode").value === "env"
      ? { env: secretValue }
      : { value: secretValue };
    if (authType === "bearer") {
      auth = { type: "bearer", secret };
    } else {
      auth = {
        type: "api_key",
        secret,
        name: byId("api-key-name").value.trim(),
        location: byId("api-key-location").value,
        prefix: byId("api-key-prefix").value.trim() || null,
      };
    }
  }

  return {
    name: byId("source-name").value.trim(),
    schema: byId("source-schema").value.trim(),
    remoteSchema: "api",
    specUrl: inline ? null : specUrl,
    specJson: inline || null,
    baseUrl: byId("base-url").value.trim() || null,
    methods,
    includeAttrs: byId("include-attrs").checked,
    tables,
    auth,
    headers: jsonObjectField("headers-json", "Static headers"),
    headersEnv: jsonObjectField("headers-env-json", "Environment-backed headers"),
    settings: {
      allowHttp: byId("allow-http").checked,
      specWithAuth: byId("spec-with-auth").checked,
      userAgent: byId("user-agent").value.trim() || null,
      connectTimeoutMs: 5000,
      requestTimeoutMs: Number(byId("request-timeout").value),
      maxResponseBytes: 52428800,
      maxPages: Number(byId("max-pages").value),
      maxRetries: 2,
    },
  };
}

function selectedTables() {
  return Array.from(document.querySelectorAll("#discovered-tables input[type=checkbox]:checked"))
    .map((input) => input.value);
}

function updateAuthFields() {
  const authType = byId("auth-type").value;
  const enabled = authType !== "none";
  document.querySelectorAll(".auth-field").forEach((element) => {
    element.classList.toggle("hidden", !enabled);
  });
  byId("api-key-fields").classList.toggle("hidden", authType !== "api_key");
  byId("secret-value").required = enabled;
  updateSecretMode();
}

function updateSecretMode() {
  const environment = byId("secret-mode").value === "env";
  byId("secret-label").textContent = environment ? "Environment variable" : "Secret value";
  byId("secret-value").type = environment ? "text" : "password";
  byId("secret-value").placeholder = environment ? "VENDOR_API_TOKEN" : "Secret is stored in pg_foreign_server";
  byId("secret-help").textContent = environment
    ? "Recommended: configure the same variable on the PostgreSQL service."
    : "Literal values work immediately but are stored unencrypted in PostgreSQL catalogs.";
}

async function loadState() {
  const status = byId("connection-status");
  status.textContent = "Connecting…";
  status.className = "status-pill status-loading";
  try {
    state.control = await api("/api/v1/state");
    renderState();
    status.textContent = state.control.extensionVersion
      ? `FDW ${state.control.extensionVersion}`
      : "PostgreSQL ready";
    status.className = "status-pill status-ready";
    clearNotice();
  } catch (error) {
    status.textContent = "Unavailable";
    status.className = "status-pill status-error";
    notice(error.message, true);
  }
}

function renderState() {
  const sources = state.control.sources || [];
  const tableCount = sources.reduce((total, source) => total + source.tables.length, 0);
  byId("source-count").textContent = String(sources.length);
  byId("table-count").textContent = String(tableCount);
  byId("pg-version").textContent = state.control.postgresVersion.split(".")[0];

  const container = byId("sources");
  container.replaceChildren();
  if (!sources.length) {
    container.append(make("div", "empty-state", "No sources yet. Add an OpenAPI document to begin."));
    return;
  }
  sources.forEach((source) => container.append(renderSource(source)));
}

function renderSource(source) {
  const card = make("article", "source-card");
  const summary = make("div", "source-summary");
  const identity = make("div");
  const title = make("div", "source-title");
  title.append(make("h3", "", source.name));
  if (source.managed) title.append(make("span", "managed-chip", "managed"));
  identity.append(title);
  const origin = source.options.spec_url || source.options.base_url || "Manual configuration";
  identity.append(make("span", "source-origin", origin));

  const actions = make("div", "source-actions");
  const sqlButton = make("button", "button button-quiet", "Copy connection SQL");
  sqlButton.type = "button";
  sqlButton.addEventListener("click", () => {
    copyConnectionSql(source).catch((error) => toast(error.message, true));
  });
  const deleteButton = make("button", "button button-quiet", "Remove");
  deleteButton.type = "button";
  deleteButton.addEventListener("click", () => removeSource(source));
  actions.append(sqlButton, deleteButton);
  summary.append(identity, actions);
  card.append(summary);

  const list = make("div", "table-list");
  source.tables.forEach((table) => {
    const row = make("div", "table-row");
    const tableIdentity = make("div", "table-name");
    tableIdentity.append(make("span", "table-method", table.method), document.createTextNode(`${table.schema}.${table.name}`));
    row.append(tableIdentity, make("div", "table-endpoint", table.endpoint));
    const browse = make("button", "button button-quiet", "Browse live rows");
    browse.type = "button";
    browse.addEventListener("click", () => openSample(source, table));
    row.append(browse);
    list.append(row);
  });
  if (!source.tables.length) list.append(make("div", "empty-state", "This server has no foreign tables."));
  card.append(list);
  return card;
}

async function copyConnectionSql(source) {
  const schemas = Array.from(new Set(source.tables.map((table) => table.schema)));
  const quoteIdentifier = (value) => `"${value.replaceAll('"', '""')}"`;
  const searchPath = schemas.length ? schemas.map(quoteIdentifier).join(", ") : "public";
  const firstSchema = schemas.length ? quoteIdentifier(schemas[0]) : "public";
  const firstTable = quoteIdentifier(source.tables[0]?.name || "table_name");
  const sql = `-- Run in any PostgreSQL client\nSET search_path TO ${searchPath}, public;\n\n-- Every SELECT below performs a live HTTP request\nSELECT * FROM ${firstSchema}.${firstTable} LIMIT 20;`;
  await navigator.clipboard.writeText(sql);
  toast("Connection SQL copied.");
}

async function removeSource(source) {
  const confirmation = window.prompt(
    `Remove foreign server “${source.name}” and all of its foreign tables? Schemas are preserved.\n\nType ${source.name} to confirm.`,
  );
  if (confirmation === null) return;
  try {
    const result = await api(`/api/v1/sources/${encodeURIComponent(source.name)}`, {
      method: "DELETE",
      body: { confirm: confirmation },
    });
    toast(result.message);
    await loadState();
  } catch (error) {
    toast(error.message, true);
  }
}

async function discover(event) {
  event.preventDefault();
  const form = byId("source-form");
  if (!form.reportValidity()) return;
  const button = byId("discover-button");
  setBusy(button, true, "Reading OpenAPI…");
  try {
    const source = sourceFromForm();
    state.discovery = {
      source,
      result: await api("/api/v1/discover", { method: "POST", body: source }),
    };
    renderDiscovery();
    byId("discovery-dialog").showModal();
  } catch (error) {
    notice(error.message, true);
    toast("Discovery failed. See the error above.", true);
  } finally {
    setBusy(button, false);
  }
}

function renderDiscovery() {
  const result = state.discovery.result;
  byId("discovery-summary").textContent = `${result.tables.length} table${result.tables.length === 1 ? "" : "s"} discovered. Uncheck operations you do not want to expose.`;
  const picker = byId("discovered-tables");
  picker.replaceChildren();
  result.tables.forEach((table) => {
    const label = make("label", "table-choice");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.value = table.name;
    checkbox.checked = true;
    checkbox.addEventListener("change", refreshSourcePlan);
    label.append(
      checkbox,
      make("span", "table-name", table.name),
      make("span", "table-endpoint", table.endpoint),
      make("span", "column-count", `${table.columns.length} columns`),
    );
    picker.append(label);
  });
  byId("sql-preview").textContent = result.sql;
  byId("apply-source-button").disabled = false;
  refreshSourcePlan();
}

async function refreshSourcePlan() {
  const tables = selectedTables();
  byId("apply-source-button").disabled = tables.length === 0;
  if (!tables.length) {
    byId("sql-preview").textContent = "Select at least one table.";
    return;
  }
  try {
    const result = await api("/api/v1/sources/plan", {
      method: "POST",
      body: {
        source: sourceFromForm(tables),
        replace: byId("replace-source").checked,
      },
    });
    byId("sql-preview").textContent = result.sql;
  } catch (error) {
    byId("sql-preview").textContent = error.message;
  }
}

async function applySource() {
  const button = byId("apply-source-button");
  setBusy(button, true, "Creating tables…");
  try {
    const result = await api("/api/v1/sources", {
      method: "POST",
      body: {
        source: sourceFromForm(selectedTables()),
        replace: byId("replace-source").checked,
      },
    });
    byId("sql-preview").textContent = result.sql;
    byId("discovery-dialog").close();
    toast(result.message);
    await loadState();
  } catch (error) {
    toast(error.message, true);
    byId("sql-preview").textContent = error.message;
  } finally {
    setBusy(button, false);
  }
}

function openSample(source, table) {
  state.sample = { source, table };
  byId("sample-title").textContent = `${table.schema}.${table.name}`;
  const select = byId("sample-column");
  select.replaceChildren();
  const none = document.createElement("option");
  none.value = "";
  none.textContent = "No filter";
  select.append(none);
  table.columns.forEach((column) => {
    const option = document.createElement("option");
    option.value = column.name;
    option.textContent = `${column.name} · ${column.dataType}`;
    select.append(option);
  });
  const placeholder = /\{([^}]+)\}/.exec(table.endpoint)?.[1];
  if (placeholder && table.columns.some((column) => column.name === placeholder)) {
    select.value = placeholder;
    byId("sample-value").placeholder = `Required: ${placeholder}`;
  } else {
    byId("sample-value").placeholder = "Optional equality value";
  }
  byId("sample-value").value = placeholder === "cep" ? "01001000" : "";
  byId("sample-sql").textContent = "Choose an optional filter, then run the live query.";
  byId("sample-result").replaceChildren(make("div", "empty-state", "No request has been made yet."));
  byId("sample-dialog").showModal();
}

async function runSample() {
  const button = byId("run-sample-button");
  const { source, table } = state.sample;
  const column = byId("sample-column").value;
  const value = byId("sample-value").value;
  if (column && !value) {
    toast("Enter a filter value or choose “No filter”.", true);
    return;
  }
  const query = new URLSearchParams({ limit: byId("sample-limit").value });
  if (column) {
    query.set("filterColumn", column);
    query.set("filterValue", value);
  }
  setBusy(button, true, "Fetching…");
  try {
    const result = await api(
      `/api/v1/sources/${encodeURIComponent(source.name)}/tables/${encodeURIComponent(table.schema)}/${encodeURIComponent(table.name)}/rows?${query}`,
    );
    byId("sample-sql").textContent = result.sql;
    renderRows(result.rows);
    toast(`${result.rows.length} live row${result.rows.length === 1 ? "" : "s"} returned.`);
  } catch (error) {
    byId("sample-result").replaceChildren(make("div", "empty-state", error.message));
    toast(error.message, true);
  } finally {
    setBusy(button, false);
  }
}

function renderRows(rows) {
  const container = byId("sample-result");
  container.replaceChildren();
  if (!rows.length) {
    container.append(make("div", "empty-state", "The API returned no matching rows."));
    return;
  }
  const columns = Array.from(new Set(rows.flatMap((row) => Object.keys(row))));
  const table = document.createElement("table");
  const head = document.createElement("thead");
  const headerRow = document.createElement("tr");
  columns.forEach((column) => headerRow.append(make("th", "", column)));
  head.append(headerRow);
  const body = document.createElement("tbody");
  rows.forEach((row) => {
    const tr = document.createElement("tr");
    columns.forEach((column) => {
      const value = row[column];
      const text = value === null || value === undefined
        ? "NULL"
        : typeof value === "object" ? JSON.stringify(value) : String(value);
      const cell = make("td", "", text);
      cell.title = text;
      tr.append(cell);
    });
    body.append(tr);
  });
  table.append(head, body);
  container.append(table);
}

async function exportSetup() {
  try {
    const response = await fetch("/api/v1/export", { credentials: "same-origin" });
    if (response.status === 401) {
      window.location.assign("/login");
      return;
    }
    if (!response.ok) throw new Error("Could not export configuration");
    const blob = await response.blob();
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = "openapi-fdw-setup.json";
    document.body.append(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(link.href);
    toast("Redacted setup bundle exported.");
  } catch (error) {
    toast(error.message, true);
  }
}

function openImport() {
  invalidateImportPlan();
  byId("import-dialog").showModal();
}

function invalidateImportPlan() {
  state.importBundle = null;
  byId("import-plan-panel").classList.add("hidden");
  byId("apply-import-button").disabled = true;
}

async function planImport() {
  invalidateImportPlan();
  try {
    const bundle = JSON.parse(byId("import-json").value);
    const replace = byId("replace-import").checked;
    const result = await api(`/api/v1/import/plan?replace=${replace}`, {
      method: "POST",
      body: bundle,
    });
    state.importBundle = { bundle, replace };
    byId("import-sql").textContent = result.sql;
    byId("import-plan-panel").classList.remove("hidden");
    byId("apply-import-button").disabled = false;
    toast(result.message);
  } catch (error) {
    invalidateImportPlan();
    toast(error.message, true);
  }
}

async function applyImport() {
  if (!state.importBundle) return;
  const button = byId("apply-import-button");
  setBusy(button, true, "Applying…");
  try {
    const { bundle, replace } = state.importBundle;
    const result = await api(`/api/v1/import/apply?replace=${replace}`, {
      method: "POST",
      body: bundle,
    });
    byId("import-dialog").close();
    toast(result.message);
    await loadState();
  } catch (error) {
    toast(error.message, true);
  } finally {
    setBusy(button, false);
  }
}

function loadImportFile(event) {
  const [file] = event.target.files;
  if (!file) return;
  file.text().then((text) => {
    byId("import-json").value = text;
    invalidateImportPlan();
  }).catch((error) => toast(error.message, true));
}

function updateSpecMode() {
  byId("spec-url").required = byId("spec-json").value.trim() === "";
}

document.addEventListener("DOMContentLoaded", () => {
  byId("source-form").addEventListener("submit", discover);
  byId("auth-type").addEventListener("change", updateAuthFields);
  byId("secret-mode").addEventListener("change", updateSecretMode);
  byId("spec-json").addEventListener("input", updateSpecMode);
  byId("apply-source-button").addEventListener("click", applySource);
  byId("run-sample-button").addEventListener("click", runSample);
  byId("refresh-button").addEventListener("click", loadState);
  byId("export-button").addEventListener("click", exportSetup);
  byId("import-button").addEventListener("click", openImport);
  byId("plan-import-button").addEventListener("click", planImport);
  byId("apply-import-button").addEventListener("click", applyImport);
  byId("import-file").addEventListener("change", loadImportFile);
  byId("import-json").addEventListener("input", invalidateImportPlan);
  byId("replace-import").addEventListener("change", invalidateImportPlan);
  updateAuthFields();
  updateSpecMode();
  loadState();
});
