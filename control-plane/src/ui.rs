use maud::{DOCTYPE, Markup, html};

const PRODUCT_NAME: &str = "OpenAPI FDW";

pub fn login(error: Option<&str>) -> String {
    shell(
        "Sign in",
        html! {
            main class="login-shell" {
                section class="login-card" aria-labelledby="login-title" {
                    div class="brand brand-large" {
                        span class="brand-mark" aria-hidden="true" { "↗" }
                        span { (PRODUCT_NAME) }
                    }
                    p class="eyebrow" { "CONTROL PLANE" }
                    h1 id="login-title" { "Connect APIs to PostgreSQL." }
                    p class="lede" {
                        "Use the administrator token configured for this control-plane service."
                    }
                    @if let Some(message) = error {
                        div class="notice notice-error" role="alert" { (message) }
                    }
                    form method="post" action="/login" class="stack" {
                        label for="token" { "Administrator token" }
                        input id="token" name="token" type="password" required autofocus
                            autocomplete="current-password" placeholder="••••••••••••••••";
                        button class="button button-primary button-wide" type="submit" { "Open control plane" }
                    }
                    p class="fine-print" {
                        "The token is checked in memory and is never stored in PostgreSQL or browser storage."
                    }
                }
            }
        },
        false,
    )
}

pub fn dashboard() -> String {
    shell(
        "Control plane",
        html! {
            header class="topbar" {
                a class="brand" href="/" aria-label="OpenAPI FDW home" {
                    span class="brand-mark" aria-hidden="true" { "↗" }
                    span { (PRODUCT_NAME) }
                }
                div class="topbar-actions" {
                    span id="connection-status" class="status-pill status-loading" { "Connecting…" }
                    button id="import-button" class="button button-quiet" type="button" { "Import setup" }
                    button id="export-button" class="button button-quiet" type="button" { "Export setup" }
                    form method="post" action="/logout" {
                        button class="button button-quiet" type="submit" { "Sign out" }
                    }
                }
            }

            main class="page-shell" {
                section class="hero" aria-labelledby="page-title" {
                    div {
                        p class="eyebrow" { "LIVE DATA PLANE" }
                        h1 id="page-title" { "Turn an OpenAPI service into tables." }
                        p class="lede" {
                            "Discover operations, review the SQL, import a schema, then query the remote API from PostgreSQL."
                        }
                    }
                    div class="metrics" aria-label="Control-plane summary" {
                        div class="metric" { strong id="source-count" { "—" } span { "sources" } }
                        div class="metric" { strong id="table-count" { "—" } span { "tables" } }
                        div class="metric" { strong id="pg-version" { "—" } span { "PostgreSQL" } }
                    }
                }

                div id="global-notice" class="notice hidden" role="status" aria-live="polite" {}

                section class="layout-grid" {
                    article class="panel setup-panel" aria-labelledby="setup-title" {
                        div class="panel-heading" {
                            div {
                                p class="eyebrow" { "01 · CONNECT" }
                                h2 id="setup-title" { "Add an API source" }
                            }
                            span class="step-chip" { "No SQL required" }
                        }
                        form id="source-form" class="form-grid" {
                            div class="field" {
                                label for="source-name" { "Source name" }
                                input id="source-name" name="name" value="brasilapi" required
                                    pattern="[a-z_][a-z0-9_]{0,62}";
                                small { "Becomes the PostgreSQL foreign server name." }
                            }
                            div class="field" {
                                label for="source-schema" { "PostgreSQL schema" }
                                input id="source-schema" name="schema" value="brasil" required
                                    pattern="[a-z_][a-z0-9_]{0,62}";
                            }
                            div class="field field-wide" {
                                label for="spec-url" { "OpenAPI document URL" }
                                input id="spec-url" name="specUrl" type="url" required
                                    value="https://raw.githubusercontent.com/sabino/openapi_fdw/main/examples/brasilapi.openapi.yaml";
                                small { "OpenAPI 3.0 or 3.1, JSON or YAML, over HTTPS." }
                            }
                            details class="field-wide advanced" {
                                summary { "Inline document or base URL override" }
                                div class="form-grid nested-grid" {
                                    div class="field field-wide" {
                                        label for="spec-json" { "Inline OpenAPI document" }
                                        textarea id="spec-json" name="specJson" rows="4"
                                            placeholder="Paste JSON or YAML instead of using a URL" {}
                                    }
                                    div class="field field-wide" {
                                        label for="base-url" { "API base URL override" }
                                        input id="base-url" name="baseUrl" type="url"
                                            placeholder="https://api.example.com/v1";
                                    }
                                }
                            }
                            fieldset class="field field-wide inline-options" {
                                legend { "Operations" }
                                label class="check" { input id="method-get" type="checkbox" checked; span { "GET" } }
                                label class="check" { input id="method-post" type="checkbox"; span { "Read-only POST" } }
                                label class="check" { input id="include-attrs" type="checkbox" checked; span { "Include full attrs JSONB" } }
                            }
                            div class="field" {
                                label for="auth-type" { "Authentication" }
                                select id="auth-type" name="authType" {
                                    option value="none" { "None" }
                                    option value="bearer" { "Bearer token" }
                                    option value="api_key" { "API key" }
                                }
                            }
                            div id="secret-mode-field" class="field auth-field hidden" {
                                label for="secret-mode" { "Secret source" }
                                select id="secret-mode" {
                                    option value="env" { "PostgreSQL environment variable" }
                                    option value="literal" { "Store literal in PostgreSQL catalog" }
                                }
                            }
                            div id="secret-field" class="field field-wide auth-field hidden" {
                                label id="secret-label" for="secret-value" { "Environment variable" }
                                input id="secret-value" autocomplete="off" placeholder="VENDOR_API_TOKEN";
                                small id="secret-help" { "Recommended: configure the same variable on the PostgreSQL service." }
                            }
                            div id="api-key-fields" class="field-wide form-grid nested-grid auth-field hidden" {
                                div class="field" {
                                    label for="api-key-name" { "API-key name" }
                                    input id="api-key-name" value="x-api-key";
                                }
                                div class="field" {
                                    label for="api-key-location" { "Location" }
                                    select id="api-key-location" {
                                        option value="header" { "Header" }
                                        option value="query" { "Query parameter" }
                                    }
                                }
                                div class="field field-wide" {
                                    label for="api-key-prefix" { "Optional prefix" }
                                    input id="api-key-prefix" placeholder="Token";
                                }
                            }
                            details class="field-wide advanced" {
                                summary { "Custom request headers" }
                                div class="form-grid nested-grid" {
                                    div class="field field-wide" {
                                        label for="headers-env-json" { "Environment-backed headers (JSON)" }
                                        textarea id="headers-env-json" rows="3" spellcheck="false" {
                                            "{}"
                                        }
                                        small { "Recommended. Map each HTTP header name to an environment-variable name, for example {\"x-tenant\":\"VENDOR_TENANT\"}." }
                                    }
                                    div class="field field-wide" {
                                        label for="headers-json" { "Literal headers (JSON)" }
                                        textarea id="headers-json" rows="3" spellcheck="false" {
                                            "{}"
                                        }
                                        small { "Literal values are stored in PostgreSQL catalogs and redacted from previews and exports." }
                                    }
                                }
                            }
                            details class="field-wide advanced" {
                                summary { "Network and request limits" }
                                div class="form-grid nested-grid" {
                                    div class="field" {
                                        label for="user-agent" { "User-Agent" }
                                        input id="user-agent" placeholder="my-team/1.0 (contact@example.com)";
                                    }
                                    div class="field inline-field" {
                                        label class="check" { input id="allow-http" type="checkbox"; span { "Allow plain HTTP" } }
                                        small { "Only for trusted local origins." }
                                    }
                                    div class="field inline-field" {
                                        label class="check" { input id="spec-with-auth" type="checkbox"; span { "Send API credentials to spec URL" } }
                                        small { "Off by default. Enable only when the OpenAPI document itself uses the same trusted authentication." }
                                    }
                                    div class="field" {
                                        label for="request-timeout" { "Request timeout (ms)" }
                                        input id="request-timeout" type="number" min="1" max="3600000" value="30000";
                                    }
                                    div class="field" {
                                        label for="max-pages" { "Maximum pages" }
                                        input id="max-pages" type="number" min="1" max="10000" value="100";
                                    }
                                }
                            }
                            label class="check field-wide replace-check" {
                                input id="replace-source" type="checkbox";
                                span { "Replace an existing source with this name" }
                            }
                            div class="field-wide action-row" {
                                button id="discover-button" class="button button-primary" type="submit" {
                                    span { "Discover tables" }
                                    span aria-hidden="true" { "→" }
                                }
                            }
                        }
                    }

                    section class="panel sources-panel" aria-labelledby="sources-title" {
                        div class="panel-heading" {
                            div {
                                p class="eyebrow" { "02 · QUERY" }
                                h2 id="sources-title" { "Available sources" }
                            }
                            button id="refresh-button" class="button button-icon" type="button" aria-label="Refresh sources" { "↻" }
                        }
                        div id="sources" class="source-list" aria-live="polite" {
                            div class="empty-state" { "Loading PostgreSQL catalogs…" }
                        }
                    }
                }
            }

            dialog id="discovery-dialog" class="dialog dialog-wide" {
                form method="dialog" class="dialog-shell" {
                    div class="dialog-heading" {
                        div { p class="eyebrow" { "DISCOVERY RESULT" } h2 { "Choose tables to import" } }
                        button class="button button-icon" value="cancel" aria-label="Close" { "×" }
                    }
                    div id="discovery-summary" class="notice notice-info" {}
                    div id="discovered-tables" class="table-picker" {}
                    details open class="sql-panel" {
                        summary { "SQL preview" }
                        pre id="sql-preview" tabindex="0" {}
                    }
                    div class="dialog-actions" {
                        button class="button button-quiet" value="cancel" { "Cancel" }
                        button id="apply-source-button" class="button button-primary" type="button" { "Create source and tables" }
                    }
                }
            }

            dialog id="sample-dialog" class="dialog dialog-wide" {
                form method="dialog" class="dialog-shell" {
                    div class="dialog-heading" {
                        div { p class="eyebrow" { "LIVE REQUEST" } h2 id="sample-title" { "Preview rows" } }
                        button class="button button-icon" value="cancel" aria-label="Close" { "×" }
                    }
                    div class="sample-controls" {
                        div class="field" {
                            label for="sample-column" { "Equality filter" }
                            select id="sample-column" {}
                        }
                        div class="field" {
                            label for="sample-value" { "Value" }
                            input id="sample-value" placeholder="Required for path parameters";
                        }
                        div class="field field-small" {
                            label for="sample-limit" { "Limit" }
                            input id="sample-limit" type="number" min="1" max="100" value="20";
                        }
                        button id="run-sample-button" class="button button-primary align-end" type="button" { "Run query" }
                    }
                    pre id="sample-sql" class="sql-inline" {}
                    div id="sample-result" class="data-grid" {}
                }
            }

            dialog id="import-dialog" class="dialog dialog-wide" {
                form method="dialog" class="dialog-shell" {
                    div class="dialog-heading" {
                        div { p class="eyebrow" { "PORTABLE SETUP" } h2 { "Import configuration" } }
                        button class="button button-icon" value="cancel" aria-label="Close" { "×" }
                    }
                    p class="muted" { "Paste an openapi-fdw/v1 JSON bundle. Literal secrets are redacted on export and must be re-entered before applying." }
                    textarea id="import-json" class="manifest-editor" rows="14" spellcheck="false" placeholder="{ … }" {}
                    label class="check" { input id="replace-import" type="checkbox"; span { "Replace sources that already exist" } }
                    details id="import-plan-panel" class="sql-panel hidden" open {
                        summary { "SQL plan" }
                        pre id="import-sql" tabindex="0" {}
                    }
                    div class="dialog-actions" {
                        input id="import-file" type="file" accept="application/json,.json" class="visually-hidden";
                        label for="import-file" class="button button-quiet" { "Choose file" }
                        button id="plan-import-button" class="button button-quiet" type="button" { "Preview" }
                        button id="apply-import-button" class="button button-primary" type="button" disabled { "Apply bundle" }
                    }
                }
            }

            div id="toast" class="toast hidden" role="status" aria-live="polite" {}
        },
        true,
    )
}

fn shell(title: &str, body: Markup, application_script: bool) -> String {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="dark light";
                title { (title) " · " (PRODUCT_NAME) }
                link rel="stylesheet" href="/assets/app.css";
                @if application_script {
                    script src="/assets/app.js" defer {}
                }
            }
            body { (body) }
        }
    }
    .into_string()
}
