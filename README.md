# OpenAPI FDW

[![CI](https://github.com/sabino/openapi_fdw/actions/workflows/ci.yml/badge.svg)](https://github.com/sabino/openapi_fdw/actions/workflows/ci.yml)

`openapi_fdw` is a native, read-only PostgreSQL foreign data wrapper for JSON
HTTP APIs. It can create typed foreign tables from OpenAPI 3.0/3.1 documents,
or expose any JSON endpoint through one stable `jsonb` column.

The runtime is Rust, [`pgrx`](https://github.com/pgcentralfoundation/pgrx), and
[`supabase-wrappers`](https://github.com/supabase/wrappers). It does **not**
embed Python, Hy, Multicorn, or a Wasm runtime in PostgreSQL. The original Hy
prototype remains available in Git history; the reasons for replacing it are
in [the audit](docs/AUDIT.md) and [architecture decision](docs/ARCHITECTURE.md).

## Start in Docker

The image contains PostgreSQL and the extension. `PG_MAJOR` can be any version
from 14 through 18.

```bash
docker build --build-arg PG_MAJOR=18 -t openapi-fdw:pg18 .
docker run --name openapi-postgres \
  -e POSTGRES_PASSWORD=postgres \
  -p 5432:5432 \
  -d openapi-fdw:pg18
```

For a persistent local development database, the equivalent shortcut is
`POSTGRES_PASSWORD=postgres docker compose up --build -d`.

After PostgreSQL is healthy:

```bash
docker exec -i openapi-postgres \
  psql -U postgres -d postgres <<'SQL'
CREATE EXTENSION openapi_fdw;

CREATE SERVER pokeapi
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (base_url 'https://pokeapi.co/api/v2');

CREATE FOREIGN TABLE pokemon (
  name text,
  height bigint,
  weight bigint,
  attrs jsonb
)
SERVER pokeapi
OPTIONS (
  endpoint '/pokemon/{name}',
  pagination 'none',
  limit_param ''
);

SELECT name, height, attrs #>> '{types,0,type,name}' AS primary_type
FROM pokemon
WHERE name = 'ditto';
SQL
```

That `SELECT` makes a real HTTPS request. A path placeholder must have an
equality predicate, and its value is percent-encoded before it is inserted into
the URL.

## The schema-drift-friendly contract

PostgreSQL table columns cannot appear dynamically when an API adds a field:
the planner needs a fixed tuple descriptor. The low-maintenance solution is a
single JSONB column:

```sql
CREATE FOREIGN TABLE api_rows (attrs jsonb)
SERVER pokeapi
OPTIONS (endpoint '/pokemon', pagination 'none');

SELECT attrs ->> 'name'
FROM api_rows;
```

Each `attrs` value is the complete source row, so newly added keys are
queryable immediately with JSONB operators and JSONPath and require no DDL
change. Typed columns and `attrs` can coexist; imported tables include the
catch-all by default.

## Import typed tables from OpenAPI

The importer accepts OpenAPI 3.0 or 3.1 in JSON or YAML. This example imports
only PokéAPI's list operation:

```sql
CREATE SERVER pokeapi_spec
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://pokeapi.co',
    spec_url 'https://raw.githubusercontent.com/PokeAPI/pokeapi/master/openapi.yml'
  );

CREATE SCHEMA poke;

IMPORT FOREIGN SCHEMA api
  LIMIT TO (pokemon_list)
  FROM SERVER pokeapi_spec
  INTO poke
  OPTIONS (methods 'GET', include_attrs 'true');

SELECT name, attrs ->> 'url'
FROM poke.pokemon_list
LIMIT 5;
```

`IMPORT FOREIGN SCHEMA` is a typed snapshot. Re-import or apply deliberate DDL
when the published contract changes; `attrs` continues to expose fields in the
meantime. Imported identifiers are deterministic, quoted, collision-safe, and
limited to PostgreSQL's 63-byte identifier size.

The importer resolves local component `$ref` values, composition keywords,
nullable OpenAPI 3.1 types, common collection envelopes, and GeoJSON feature
properties. External `$ref` documents are intentionally not fetched.

## Manual table examples

Nested API objects can be projected before column lookup:

```sql
CREATE SERVER weather
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://api.weather.gov',
    user_agent 'my-app/1.0 (https://example.com/contact)'
  );

CREATE FOREIGN TABLE weather_point (
  point text,
  grid_id text,
  grid_x bigint,
  grid_y bigint,
  attrs jsonb
)
SERVER weather
OPTIONS (
  endpoint '/points/{point}',
  object_path '/properties',
  pagination 'none',
  limit_param ''
);
```

Column names are matched directly, through `column_map`, or by sanitized
camelCase-to-snake_case matching. `response_path` selects a row collection;
`object_path` selects the object used for typed projection inside each row.
`attrs` always retains the complete row before `object_path` is applied.

POST endpoints that are semantically reads are explicit:

```sql
CREATE FOREIGN TABLE search_results (
  id bigint,
  title text,
  attrs jsonb
)
SERVER my_api
OPTIONS (
  endpoint '/search',
  method 'POST',
  request_body '{"query":"postgres"}',
  response_path '/results'
);
```

The wrapper never implements `INSERT`, `UPDATE`, or `DELETE`.

## Options

Server options apply to the OpenAPI document and data requests.

| Option | Default | Purpose |
| --- | --- | --- |
| `base_url` | derived from spec | HTTPS origin/base path for table endpoints |
| `spec_url` / `spec_json` | none | OpenAPI document used by schema import |
| `headers` | `{}` | JSON object of static HTTP headers |
| `user_agent` | `openapi_fdw/0.2` | explicit User-Agent, required by some APIs |
| `accept` | `application/json` | Accept header |
| `api_key` | none | API key placed in a header or query parameter |
| `api_key_location` | `header` | `header` or `query` |
| `api_key_name` | `x-api-key` | header/query parameter name |
| `api_key_prefix` | none | prefix such as `Token` |
| `bearer_token` | none | `Authorization: Bearer ...` value |
| `connect_timeout_ms` | `5000` | TCP/TLS connection timeout |
| `request_timeout_ms` | `30000` | whole-request timeout |
| `max_response_bytes` | `52428800` | decompressed body limit, at most 512 MiB |
| `max_pages` | `100` | pagination safety cap |
| `max_retries` | `2` | retries for transient network/HTTP failures |
| `max_retry_delay_ms` | `5000` | exponential/`Retry-After` delay cap |
| `max_redirects` | `5` | same-origin redirect cap |
| `allow_http` | `false` | permit plaintext HTTP for trusted local services |
| `allow_cross_origin_pagination` | `false` | forward pagination to another origin |

One of `base_url`, `spec_url`, or `spec_json` is required. When only the spec is
given, the first OpenAPI server URL and its variable defaults are used.

| Foreign-table option | Default | Purpose |
| --- | --- | --- |
| `endpoint` | required | relative path, optionally with `{column}` placeholders |
| `method` | `GET` | `GET` or explicit read-only `POST` |
| `response_path` | automatic | RFC 6901 pointer to the row collection |
| `object_path` | none | pointer within each row for typed projection |
| `query_params` | `{}` | static query parameters as JSON |
| `request_body` | none | static JSON POST body |
| `column_map` | `{}` | SQL-column to JSON-name/pointer map |
| `query_param_map` | `{}` | SQL-column to API-query-name map |
| `attrs_column` | `attrs` | name of the full-row JSONB column |
| `limit_param` | `limit` | API parameter for a safe SQL limit; empty disables |
| `page_size` / `page_size_param` | none | explicit page size and parameter name |
| `cursor_path` / `cursor_param` | inferred / `cursor` | custom cursor pointer and request parameter |
| `pagination` | `auto` | `auto` or `none` |
| `max_pages` | server value | per-table page cap |
| `on_type_error` | `error` | `error` or return `null` for that cell |

Auto-pagination understands RFC 8288 `Link`, common `next` URL fields, and
cursor fields. Duplicate tokens, cross-origin URLs, and page-cap overruns fail
closed.

## SQL semantics and performance

- Simple equality predicates are sent as API query parameters. Placeholders
  consume equality predicates as path parameters. PostgreSQL still applies its
  local filter.
- `LIMIT + OFFSET` bounds the remote fetch only when there is no local filter or
  sort that could change correctness. PostgreSQL applies `OFFSET` itself.
- Sorting is local; there is no generic ordering vocabulary shared by all APIs.
- Each scan reads live remote data. There is no hidden cache.
- HTTP clients and connections are pooled per PostgreSQL backend. The backend
  remains occupied while waiting for the remote service, as with other
  synchronous FDWs.

On the recorded PG18/local-API benchmark, one typed scan with one pooled HTTP
request had a 0.836 ms median and 1.129 ms p95 (1,153 scans/s) in one backend.
Eight backends had a 3.834 ms median and 1,940 scans/s total. Direct HTTP against
the same test server averaged 0.313 ms, putting the SQL/FDW/JSON/tuple portion
near 0.6 ms in that environment. See
[the benchmark notes](docs/BENCHMARKS.md) before comparing these numbers with a
network API.

## Security and operations

Only HTTPS is accepted by default. TLS certificates are verified. Redirects,
response bodies, timeouts, retries, and pagination are bounded, and credentials
are redacted from extension-generated errors.

Creating/configuring a foreign server authorizes outbound traffic; this is not
an SSRF sandbox. API credentials are currently server options and therefore
stored unencrypted in PostgreSQL catalogs. Restrict ownership and catalog
access accordingly. The current Wrappers framework does not expose user-mapping
options to the FDW constructor, so per-role secret mappings are not yet
supported.

Complex JSONB predicates work and are evaluated locally, but the current
`supabase-wrappers` planner emits an `unsupported qual` warning for expressions
it cannot push down. This is noisy, not a correctness failure, and is tracked as
an upstream framework limitation.

## Native installation

Docker or the release images are the shortest route. For a host installation,
install the development package for the exact PostgreSQL major, Rust 1.88 or
newer, and `cargo-pgrx` 0.16.1, then run:

```bash
cargo pgrx init --pg18="$(command -v pg_config)"
cargo pgrx install --release --no-default-features --features pg18
```

Use the matching feature (`pg14` through `pg18`) and `pg_config`. A shared
library built for one PostgreSQL major must not be copied to another.

## Validation

Pull requests build and run a real PostgreSQL integration suite for versions 14
through 18. The suite starts a deterministic HTTP server and covers OpenAPI
import, JSONB drift, typed scalars/arrays, timestamps, path/query/LIMIT
behavior, pagination, GeoJSON, POST, retries, auth headers, response bounds,
content types, and hostile pagination.

An independent scheduled workflow queries PokéAPI, BrasilAPI, and the US
National Weather Service over live HTTPS. Public-service availability is never
the only merge oracle. The selection and observed contracts are recorded in
[the API research notes](docs/API_RESEARCH.md).

## License

WTFPL, as declared in `Cargo.toml`.
