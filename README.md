# OpenAPI FDW

[![CI](https://github.com/sabino/openapi_fdw/actions/workflows/ci.yml/badge.svg)](https://github.com/sabino/openapi_fdw/actions/workflows/ci.yml)

Turn an OpenAPI-described HTTP API into live PostgreSQL tables.

`openapi_fdw` is a native PostgreSQL foreign data wrapper written in Rust. It
discovers operations from OpenAPI 3.0/3.1, creates typed foreign tables, and
makes a real HTTP request whenever PostgreSQL scans one of those tables. Tables
are read-only by default and can opt into explicit POST/PUT/PATCH/DELETE
endpoints. Every table can also retain the complete source object in an `attrs
jsonb` column, so new API fields are immediately queryable without DDL.

The project ships two deliberately separate pieces:

- the **data plane**, a native PostgreSQL extension built with `pgrx` and the
  `supabase-wrappers` Rust callback adapter; and
- the **control plane**, a small stateless Rust web service that discovers
  tables, previews the exact SQL, applies configuration transactionally,
  previews live rows, and imports or exports redacted setup bundles.

There is no Supabase service dependency and no Wasm data path. The adapter is
linked into the same native extension and supplies PostgreSQL FDW callback
boilerplate. There is also no Python interpreter, Hy runtime, Multicorn process,
Node server, or hidden data-copy service in production.

## Run the complete stack

```bash
cp .env.example .env
# Put independent random values in POSTGRES_PASSWORD and
# OPENAPI_FDW_ADMIN_TOKEN, then:
docker compose up --build -d
```

Open [http://localhost:8080](http://localhost:8080), sign in with
`OPENAPI_FDW_ADMIN_TOKEN`, and press **Discover tables**. The form is already
filled with this repository's directly importable BrasilAPI definition.

Choose one or more operations, inspect the generated SQL, and apply it. A few
seconds later PostgreSQL has ordinary-looking foreign tables such as:

```sql
SELECT code, name, full_name, attrs ->> 'logo_url' AS logo_url
FROM brasil.banks
WHERE code IS NOT NULL
ORDER BY code
LIMIT 20;

SELECT nome, valor
FROM brasil.interest_rates;

SELECT cep, city, state, attrs #>> '{location,coordinates,latitude}' AS latitude
FROM brasil.address_by_cep
WHERE cep = '01001000';
```

Those queries do not read a stale import or replica. They issue bounded live
HTTPS requests to BrasilAPI and turn the JSON response into PostgreSQL rows.

## How the pieces fit

```text
browser ──> control plane ──SQL──> PostgreSQL + openapi_fdw
                                      │
SQL clients ──────PostgreSQL──────────┤
                                      └──HTTPS──> external JSON APIs
```

The control plane can be stopped after configuration. PostgreSQL keeps serving
the foreign tables because the data plane has no dependency on the web app.
Any PostgreSQL-compatible client can query the tables without an OpenAPI-aware
driver or plugin.

## Use any compatible OpenAPI document

The normal control-plane flow is:

1. Enter an HTTPS URL to an OpenAPI 3.0 or 3.1 JSON/YAML document, or paste the
   document inline.
2. Optionally override its API base URL and configure authentication.
3. Discover the GET operations, plus explicitly enabled POST scans. Optionally
   ask the importer to pair compatible POST/PATCH/PUT/DELETE operations as
   writable table capabilities.
4. Select tables and inspect the redacted `CREATE SERVER` and
   `IMPORT FOREIGN SCHEMA` statements.
5. Apply the transaction, then preview live rows before connecting another
   PostgreSQL client.

BrasilAPI's `/docs` page is assembled from OpenAPI fragments at render time and
does not expose a stable raw specification URL. The project therefore includes
a small, reviewable [BrasilAPI OpenAPI document](examples/brasilapi.openapi.yaml)
for its banks, rates, brokers, and CEP endpoints. It uses BrasilAPI's real
public contracts and correct `/api` base path.

## Schema evolution without table churn

PostgreSQL plans queries against a fixed tuple descriptor, so no FDW can safely
invent typed columns halfway through a query when an upstream API adds a key.
OpenAPI FDW uses two complementary contracts:

- `IMPORT FOREIGN SCHEMA` creates a useful typed snapshot of the published
  contract.
- `attrs jsonb` contains the complete source row and exposes new or nested keys
  immediately.

```sql
SELECT
  attrs ->> 'newField',
  attrs #>> '{nested,value}'
FROM vendor.items;
```

For an intentionally untyped endpoint, declare only `attrs jsonb`; no OpenAPI
document is required.

## Opt-in writes to HTTP APIs

Writable tables declare each allowed operation. Omitting an endpoint keeps that
SQL operation disabled:

```sql
CREATE FOREIGN TABLE app.items (
  id text,
  name text,
  data jsonb,
  attrs jsonb
)
SERVER vendor
OPTIONS (
  endpoint '/objects',
  rowid_column 'id',
  rowid_parameter 'objectId',
  insert_endpoint '/objects',
  insert_method 'POST',
  update_endpoint '/objects/{objectId}',
  update_method 'PATCH',
  delete_endpoint '/objects/{objectId}',
  write_columns '["name","data"]'
);

INSERT INTO app.items (name, data)
VALUES ('From PostgreSQL', '{"status":"new"}');

UPDATE app.items SET data = '{"status":"ready"}' WHERE id = 'object-id';
DELETE FROM app.items WHERE id = 'object-id';
```

`write_columns` is an optional body whitelist; without it, all typed columns
present in the modification row except the row identity and `attrs` are eligible
to be sent. `write_mode 'attrs'` sends one
JSONB object as the complete request body. `write_mode 'merge'` starts with that
JSONB object and overlays the selected typed columns. `column_map` applies in
both directions, including object-only JSON Pointer paths.

HTTP side effects cannot participate in a PostgreSQL transaction: a later SQL
rollback cannot undo an accepted remote request, and a multi-row statement can
partially succeed. Mutations execute one request per row. POST and PATCH are
never retried automatically because their effects may not be idempotent. The
current callback adapter also does not support `RETURNING`. Use a remote API
with stable row identities and design write workflows around these boundaries.
See [writable table behavior](docs/WRITES.md) for the full contract.

For a compatible OpenAPI document, the declarative path is one extra flag:

```sql
IMPORT FOREIGN SCHEMA api
  FROM SERVER vendor
  INTO app
  OPTIONS (methods 'GET', include_attrs 'true', writable 'true');
```

For each GET collection, the importer looks for POST on the collection and
PATCH (preferred) or PUT plus DELETE on a one-identity item path. It derives the
row identity and body whitelist from path parameters and JSON request schemas.
Operations that cannot be paired safely stay read-only.

The repository includes a directly importable contract for the public
[RESTful API.dev object service](examples/restful-api.openapi.yaml). Its
documented anonymous API supports POST, GET, PUT, PATCH, and DELETE with a small
daily request allowance. The manual
[`Live public CRUD validation`](.github/workflows/live-crud.yml) creates one
disposable object, imports `get_object`, performs PATCH/GET/DELETE through
PostgreSQL, and always attempts cleanup. Local PostgreSQL 14-18 tests cover SQL
INSERT as well because the public service's generated ID cannot currently be
obtained through `RETURNING`.

## Authentication and portable setup bundles

The FDW understands bearer tokens, header/query API keys, static headers, and
custom user agents. Secrets can be literal PostgreSQL server options, but the
recommended form names an environment variable on the PostgreSQL service:

```sql
CREATE SERVER vendor
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    spec_url 'https://vendor.example/openapi.json',
    bearer_token_env 'VENDOR_API_TOKEN'
  );
```

Equivalent options exist for `api_key_env` and `headers_env`. The environment
variable is resolved inside the PostgreSQL process and its value is redacted
from extension errors. API credentials are not sent to the OpenAPI document URL
unless `spec_with_auth 'true'` is explicitly configured for a trusted private
specification.

Control-plane exports use the `openapi-fdw/v1` JSON format. Environment
references remain portable. Literal secret values are replaced with a
`configured` marker and must be re-entered before applying the bundle on a new
instance. See [control-plane configuration](docs/CONTROL_PLANE.md).

## SQL-only operation

The web control plane is optional. Everything remains normal PostgreSQL DDL:

```sql
CREATE EXTENSION openapi_fdw;

CREATE SERVER pokeapi
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    spec_url 'https://raw.githubusercontent.com/PokeAPI/pokeapi/master/openapi.yml'
  );

CREATE SCHEMA poke;
IMPORT FOREIGN SCHEMA api
  LIMIT TO (pokemon_list)
  FROM SERVER pokeapi
  INTO poke
  OPTIONS (methods 'GET', include_attrs 'true');

SELECT name, attrs ->> 'url'
FROM poke.pokemon_list
LIMIT 5;
```

Manual JSONB-only tables are equally small:

```sql
CREATE SERVER weather
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://api.weather.gov',
    user_agent 'my-team/1.0 (contact@example.com)'
  );

CREATE FOREIGN TABLE weather_point (attrs jsonb)
  SERVER weather
  OPTIONS (
    endpoint '/points/{point}',
    object_path '/properties',
    pagination 'none',
    limit_param ''
  );

SELECT attrs #>> '{properties,gridId}'
FROM weather_point
WHERE point = '39.7456,-97.0892';
```

## Installation choices

### Slim PostgreSQL image

Release images use the official Alpine PostgreSQL base and contain only
PostgreSQL, CA certificates, and the native extension:

```bash
docker run --name openapi-postgres \
  -e POSTGRES_PASSWORD="$(openssl rand -hex 24)" \
  -p 5432:5432 \
  -v openapi-fdw-data:/var/lib/postgresql \
  -d ghcr.io/sabino/openapi_fdw:pg18
```

Tags `pg14` through `pg18` track the latest release for each PostgreSQL major.
Versioned tags use `v0.4.0-pg18`. The control plane is a separate stripped
scratch image:

```text
ghcr.io/sabino/openapi_fdw:control
```

### Checksummed native package

On glibc Linux x86-64 with PostgreSQL development/runtime files installed:

```bash
curl -fsSL https://raw.githubusercontent.com/sabino/openapi_fdw/main/scripts/install.sh \
  | sudo sh -s -- --version v0.4.0 --pg-config /usr/lib/postgresql/18/bin/pg_config
```

The installer detects the PostgreSQL major, downloads its release archive,
verifies SHA-256, and copies only the extension library, control file, and SQL
file into the directories reported by `pg_config`.

### Build from source

Install Rust 1.88+, `cargo-pgrx` 0.16.1, libclang, and the development package
for the exact PostgreSQL major, then:

```bash
cargo pgrx init --pg18="$(command -v pg_config)"
cargo pgrx install --release --no-default-features --features pg18
```

A library built for one PostgreSQL major or C library must not be copied to an
incompatible server. More detail is in [installation and deployment](docs/DEPLOYMENT.md).

## HTTP and SQL behavior

- Endpoints with path placeholders are parameterized lookup relations. A scan
  issues its request only when every placeholder has an equality predicate,
  such as `WHERE cep = '01001000'`. An unbound scan returns no rows and makes
  no HTTP request, allowing catalog and BI clients to discover the declared
  columns safely.
- OpenAPI 3.0/3.1 JSON or YAML, local component `$ref`, compositions, arrays,
  common envelopes, and GeoJSON are supported.
- GET and explicitly configured POST scans are supported. Explicit mutation
  endpoints map SQL INSERT/UPDATE/DELETE to POST or PUT, PATCH or PUT, and
  DELETE respectively; tables remain read-only by default.
- HTTPS certificate verification is on by default. Plain HTTP requires an
  explicit opt-in.
- Connections are pooled per PostgreSQL backend. Timeouts, decompressed body
  size, redirects, retries, pagination pages, and retry delay are bounded.
- Same-origin redirects and pagination are enforced by default.
- Path placeholders and simple equality predicates become path/query
  parameters. PostgreSQL still retains local correctness checks.
- Remote `LIMIT` is used only when local filters or sorting cannot change the
  result. Sorting remains local.
- Each scan reads the remote service live. There is no hidden cache or local
  dataset unless the user deliberately materializes a query.

The complete option reference and security boundaries are in
[the architecture document](docs/ARCHITECTURE.md).

## Performance and validation

The deterministic suite loads the actual extension, starts a real HTTP origin,
and runs PostgreSQL integration tests on versions 14 through 18. A separate
control-plane test covers authentication, discovery, SQL preview, transactional
DDL, live row browsing, export, and removal. Scheduled smoke tests query
PokéAPI, BrasilAPI, and the US National Weather Service over public HTTPS.

On the recorded PostgreSQL 18/local-origin benchmark, one typed scan had a
0.836 ms median and 1.129 ms p95 at 1,153 scans/s in one backend. Eight backends
reached 1,940 scans/s. Public network latency normally dominates; see
[benchmark methodology and results](docs/BENCHMARKS.md).

## Documentation

- [Control plane and bundle format](docs/CONTROL_PLANE.md)
- [Installation and containers](docs/DEPLOYMENT.md)
- [Architecture and trade-offs](docs/ARCHITECTURE.md)
- [Writable table behavior](docs/WRITES.md)
- [Original prototype audit](docs/AUDIT.md)
- [Public API research](docs/API_RESEARCH.md)
- [Benchmarks](docs/BENCHMARKS.md)

## License

WTFPL, as declared in `Cargo.toml`.
