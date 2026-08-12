# Architecture decision: native Rust OpenAPI FDW

Status: accepted for the overhaul

Decision date: 2026-08-12

## Outcome

Replace the Hy/Multicorn implementation with a native Rust PostgreSQL extension
built on the maintained
[`supabase-wrappers`](https://github.com/supabase/wrappers/tree/main/supabase-wrappers)
FDW framework and
[`pgrx`](https://github.com/pgcentralfoundation/pgrx).

The extension will remain read-only initially. It will expose both:

- typed columns, declared manually or generated as a point-in-time snapshot by
  `IMPORT FOREIGN SCHEMA`; and
- an `attrs jsonb` catch-all containing the complete source object for schema
  evolution and nested-field access.

The Hy implementation remains available in Git history as an educational
prototype, but it will not be installed in a PostgreSQL backend.

## Why the relational schema cannot be fully dynamic

PostgreSQL stores every foreign table column and type in its catalogs just like
a local table. A query is planned against that fixed tuple descriptor. An API
adding a field cannot safely make a new typed SQL column appear in the middle of
an already planned query.

There are two honest interfaces:

1. `IMPORT FOREIGN SCHEMA` reads an OpenAPI document and creates a typed schema
   snapshot. Re-run a deliberate migration when that contract changes.
2. An `attrs jsonb` column stays stable while its keys evolve. PostgreSQL 18's
   JSONB operators and JSONPath can query nested fields without changing DDL.

The extension will combine them. Imported and manually declared tables include
`attrs jsonb` by default, so a newly added API field is immediately queryable
even before typed DDL is updated.

## Options considered

| Option | Runtime performance | Install/operations | API and PostgreSQL fit | Decision |
| --- | --- | --- | --- | --- |
| Hy on Multicorn | Python-level; HTTP usually dominates | Requires an in-backend Python ABI plus a Multicorn build; current Multicorn work ends at the PostgreSQL 13 era | Pleasant source language, but the framework is stale and the current image installs the wrong PyPI package | Reject for production |
| C or C++ | Highest possible native throughput | Manual PostgreSQL ABI, memory, exception, TLS, and package work for every major version | Excellent libraries exist, but unsafe failure modes live inside the database process | Reject |
| Go | Fast application HTTP stack | Embedding the Go runtime and GC through a C ABI in each PostgreSQL backend complicates lifecycle and packaging | No mature FDW framework comparable to pgrx/Wrappers | Reject |
| Zig | Native C ABI and small artifacts | Young PostgreSQL and HTTP ecosystem; most FDW machinery would be handwritten | Attractive experiment, high maintenance risk | Reject |
| Raw Rust/pgrx | Native and memory-safe at the language boundary | Per-major packages and pgrx build tooling | Strong choice, but requires reimplementing planner/FDW boilerplate | Viable fallback |
| Rust with `supabase-wrappers` | Native; no interpreter or Wasm startup | Per-major packages, with the planner/scan/import framework supplied | Best balance of safety, planner integration, HTTP ecosystem, and maintainability | **Choose** |
| Supabase OpenAPI Wasm FDW | Feature-rich and sandboxed | Easy guest upgrades on an installed Wrappers host | Excellent fallback and reference behavior, but its own benchmark reports about 170-180 ms fixed startup per scan | Keep as reference/fallback, not the performance target |

Changing the surface language from Hy to Rust is not based on syntax taste.
The decisive facts are the abandoned production bridge, the need to ship across
current PostgreSQL majors, and the measurable Wasm startup cost. HTTP latency
will still dominate many public API calls, but eliminating avoidable fixed cost
matters for local and low-latency services.

## Target architecture

```text
PostgreSQL planner/executor
        |
        v
supabase-wrappers native FDW callbacks
        |
        +--> OpenAPI importer --> safe CREATE FOREIGN TABLE statements
        |
        +--> request planner --> path/query/LIMIT pushdown
        |                          |
        |                          v
        +--------------------> pooled Rust HTTP client
                                   |
                                   v
                              remote JSON API
        ^
        |
JSON decoder / type coercion / attrs jsonb row projection
```

The extension is loaded directly with `CREATE EXTENSION openapi_fdw`. It does
not require Python, Hy, Multicorn, a sidecar service, or a Wasm artifact at
runtime.

## Required behavior

### PostgreSQL integration

- PostgreSQL 14 through 18 feature builds and packages.
- Real `CREATE EXTENSION`, `CREATE SERVER`, `CREATE FOREIGN TABLE`, and
  `IMPORT FOREIGN SCHEMA` behavior.
- Equality qual pushdown to path and query parameters.
- `LIMIT` and `OFFSET` pushdown through configurable parameter names.
- Planner estimates that do not claim a local table-sized cost for a remote
  HTTP request.
- Useful `EXPLAIN VERBOSE` details without credentials.

### OpenAPI and HTTP

- OpenAPI 3.0 and 3.1 JSON or YAML documents.
- Local component `$ref` resolution with recursion limits; `allOf`, `oneOf`,
  and `anyOf` handled conservatively.
- Server URL selection and server-variable defaults.
- GET plus explicit POST-for-read endpoints.
- Static headers, API keys, bearer tokens, and credential redaction.
- Connection pooling, TLS verification, connect/request timeouts, decompression,
  bounded response bodies, retry/backoff for transient statuses, and
  `Retry-After` support.
- Array, single-object, common-envelope, and GeoJSON response normalization.
- Link-, URL-, cursor-, and offset-oriented pagination with loop and page caps.

### Schema behavior

- Deterministic and quoted table/column names.
- OpenAPI primitive/format mapping to PostgreSQL types.
- Nested objects and arrays mapped to JSONB.
- Automatic `attrs jsonb` unless explicitly disabled.
- Response-envelope and GeoJSON inference during import so generated columns
  describe actual rows, not the outer document.
- Manual JSONB-only tables for APIs that publish no OpenAPI document.

## Security boundaries

- Creating a foreign server is privileged and authorizes outbound requests to
  its configured destination. The extension is not an SSRF sandbox.
- Secrets must never appear in PostgreSQL errors, `EXPLAIN`, debug logs, or test
  artifacts.
- Server options in PostgreSQL catalogs are not encrypted. Documentation must
  prefer user mappings or an external secret facility when available and state
  the catalog visibility tradeoff when static secrets are used.
- Redirects, response size, request time, retries, and pagination are bounded.
- Dynamic DDL generated from OpenAPI uses identifier/literal quoting rather
  than string interpolation.

## Validation matrix

The deterministic suite uses a local mock server and proves that a SQL query
causes a real HTTP request. It covers direct arrays, response envelopes,
GeoJSON, path parameters, query and limit pushdown, pagination, type coercion,
JSONB catch-all behavior, errors, and schema import.

Live opt-in smoke tests use several independent public APIs:

| API | Why it is useful |
| --- | --- |
| [PokéAPI](https://github.com/PokeAPI/pokeapi/blob/master/openapi.yml) | OpenAPI 3.1, no auth, `results` envelope, offset/URL pagination, list and path-parameter detail endpoints |
| [National Weather Service](https://www.weather.gov/documentation/services-web-api) | Official live OpenAPI document, required User-Agent, GeoJSON, nested properties, cursor-like pagination |
| [BrasilAPI](https://github.com/BrasilAPI/BrasilAPI) | Real Portuguese/Brazilian data and no published OpenAPI file; validates the JSONB-only manual-table path |
| GitHub REST | Optional authenticated test for API keys, rate limits, and RFC 8288 Link pagination |

Live services are never the only CI oracle; their availability and contracts
are outside this repository's control.

## Performance acceptance

Benchmarks run against the same local mock server, payload, PostgreSQL major,
and warm/cold connection state. They report at least median and p95 for:

- raw HTTP fetch;
- JSONB-only FDW scan;
- typed projection plus `attrs`;
- schema import; and
- pagination over a fixed row count.

The native implementation must not have the roughly 170-180 ms fixed scan cost
documented by the reference Wasm implementation. Network time, HTTP decoding,
and PostgreSQL tuple construction are reported separately where practical.

## Stack plan

1. **Architecture and audit** — this decision plus reproducible baseline
   evidence.
2. **Native runtime** — Rust extension, deterministic PostgreSQL integration
   suite, and a Docker development image.
3. **Production hardening** — current-major CI/package matrix, live API smoke
   tests, benchmarks, security/operations documentation, and migration notes.

Each branch is based on the previous branch so the pull requests can be reviewed
and merged as a stack.
