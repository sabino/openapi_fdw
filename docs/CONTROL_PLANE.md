# Control plane

The control plane is an optional configuration surface for OpenAPI FDW. It is
a single statically linked Rust executable with embedded HTML, CSS, and
JavaScript. It does not proxy query traffic and it does not copy API data.

```text
administrator browser --> control plane --> PostgreSQL DDL/catalogs
                                              |
SQL clients ----------------------------------+
                                              |
                                              +--> external API on every scan
```

After a source has been applied, the service can be stopped without affecting
its foreign tables.

## Configuration

| Environment variable | Required | Default | Meaning |
| --- | --- | --- | --- |
| `DATABASE_URL` | yes | none | PostgreSQL URI for the database being managed |
| `OPENAPI_FDW_ADMIN_TOKEN` | yes | none | Administrator token, at least 16 characters |
| `OPENAPI_FDW_LISTEN` | no | `0.0.0.0:8080` | Listen address |
| `OPENAPI_FDW_COOKIE_SECURE` | no | `true` | Require HTTPS for the session cookie |
| `OPENAPI_FDW_POOL_SIZE` | no | `8` | PostgreSQL connection-pool size, 1 through 64 |
| `RUST_LOG` | no | `openapi_fdw_control=info` | Rust tracing filter |

The current control-plane PostgreSQL connection uses `NoTls`. In containers or
an orchestrator it should use a private network and `sslmode=disable`. Do not
expose that unencrypted connection across an untrusted network.

The login token is held in process memory. A successful login receives a
derived, HTTP-only, same-site session cookie; the administrator token itself is
not written to browser storage or PostgreSQL. Production deployments must keep
the secure-cookie default and terminate HTTPS in front of the service.

State-changing API requests also require
`X-OpenAPI-FDW-Request: control-plane`. The browser application sets this
header automatically. There is no permissive cross-origin policy.

## Normal workflow

1. Enter a lowercase source/server name and destination PostgreSQL schema.
2. provide an HTTPS OpenAPI 3.0/3.1 URL or paste JSON/YAML inline.
3. Optionally override the API base URL, enable POST scans, opt into declared
   upstream writes, or configure authentication and request bounds.
4. Discover operations. Discovery creates a temporary server and schema inside
   a PostgreSQL transaction, reads the resulting catalogs, and rolls the entire
   preview back.
5. Select the operations to expose and inspect the redacted SQL plan.
6. Apply. Server creation, schema creation, table import, validation, and
   control metadata are one transaction.
7. Browse a small set of live rows, optionally with one equality filter.

Replacing a source is explicit. The transaction drops that foreign server and
its foreign tables, recreates the selected contract, verifies it, and either
commits everything or restores the previous state on error. The destination
schema itself is preserved. Source removal likewise requires typing the exact
server name and preserves schemas.

The control plane records a redacted source definition in
`openapi_fdw_control.sources`. Servers created only with SQL still appear, but
are marked unmanaged because no portable definition is available.

## Authentication choices

Literal bearer tokens, API keys, and custom header values work immediately,
but PostgreSQL stores foreign-server options in its catalogs. Production
deployments should normally place secrets in the PostgreSQL service environment
and configure only their names:

```json
{
  "auth": {
    "type": "bearer",
    "secret": { "env": "VENDOR_API_TOKEN" }
  },
  "headersEnv": {
    "x-tenant": "VENDOR_TENANT_ID"
  }
}
```

The environment variable must exist and be non-empty in every PostgreSQL
container or process that can execute the foreign table. Environment-backed
secrets are process-wide, not per-role. Use PostgreSQL ownership and grants to
decide which roles may query each foreign schema.

API credentials and custom headers are not sent while fetching `specUrl` by
default. This prevents a bearer token for one API from leaking when its OpenAPI
document is hosted on another origin. A private specification can explicitly
enable `settings.specWithAuth`; do so only when the document URL is trusted to
receive the same credentials.

## Portable bundle format

Export downloads a versioned JSON document:

```json
{
  "apiVersion": "openapi-fdw/v1",
  "kind": "OpenApiFdwBundle",
  "sources": [
    {
      "name": "brasilapi",
      "schema": "brasil",
      "remoteSchema": "api",
      "specUrl": "https://example.invalid/openapi.yaml",
      "methods": ["GET"],
      "includeAttrs": true,
      "writable": false,
      "tables": ["banks", "interest_rates"],
      "auth": { "type": "none" },
      "settings": {
        "allowHttp": false,
        "connectTimeoutMs": 5000,
        "requestTimeoutMs": 30000,
        "maxResponseBytes": 52428800,
        "maxPages": 100,
        "maxRetries": 2
      }
    }
  ]
}
```

Environment-variable references remain in exports. Literal credentials are
removed and replaced by a `configured` marker; re-enter those values before
applying the bundle elsewhere. Inline OpenAPI documents are retained because
they are part of the portable table contract, so do not put credentials in an
OpenAPI document.

Import validates the complete bundle, previews redacted SQL, and applies all
sources in one transaction. Existing server names are rejected unless the
operator explicitly chooses replace.

## HTTP API

Automation can send `Authorization: Bearer <administrator token>`. All request
and response bodies are JSON unless noted otherwise.

| Method and path | Purpose |
| --- | --- |
| `GET /healthz` | Unauthenticated process and database health |
| `GET /api/v1/state` | PostgreSQL version, extension version, sources, tables, columns |
| `POST /api/v1/discover` | Transactional discovery from one source definition |
| `POST /api/v1/sources/plan` | Redacted SQL for one source |
| `POST /api/v1/sources` | Apply one source; body also carries `replace` |
| `DELETE /api/v1/sources/{name}` | Remove a server and its tables with exact confirmation |
| `GET /api/v1/sources/{source}/tables/{schema}/{table}/rows` | Fetch up to 100 live rows |
| `GET /api/v1/export` | Download a redacted bundle |
| `POST /api/v1/import/plan?replace=false` | Validate and preview a bundle |
| `POST /api/v1/import/apply?replace=false` | Transactionally apply a bundle |

The live-row endpoint accepts `limit`, plus `filterColumn` and `filterValue` as
an all-or-nothing equality filter. It verifies the requested objects are
foreign tables owned by this FDW before generating quoted SQL.

## Boundaries

- The service is an administrator tool. Do not make it anonymous or expose it
  without HTTPS and a strong independent token.
- Creating a foreign server authorizes the PostgreSQL backend to make outbound
  requests. The project bounds redirects, time, response size, retries, and
  pages, but it is not a network-level SSRF sandbox.
- SQL previews and exports are redacted. PostgreSQL superusers and operating
  system administrators can still inspect process configuration and catalogs.
- The service does not proxy or expose a direct HTTP endpoint for upstream
  mutations. When a source has `writable: true`, it asks the FDW importer to
  attach only safely paired OpenAPI mutation operations to the generated foreign
  tables. SQL users can then cause real remote side effects. PostgreSQL rollback
  cannot undo those HTTP requests; review the generated options and grants
  before applying them.
