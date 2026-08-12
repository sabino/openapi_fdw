# OpenAPI FDW audit

Audit date: 2026-08-12

This document records the baseline before the production rewrite. It is
deliberately blunt: the current repository is a useful Hy language experiment,
but it is not a working PostgreSQL OpenAPI foreign data wrapper.

## What exists today

The package contains a small Multicorn wrapper written mostly in Hy. At wrapper
construction time it downloads one OpenAPI document, looks up one configured
path and method, and remembers the response property's names. During a scan it
makes one HTTP request, requires the selected payload to be an array of JSON
objects, and projects keys into the columns requested by Multicorn.

That narrow flow works in mock-only tests. It is useful as a sketch of the
control flow, but it does not implement the behavior advertised in the README.

## Blocking defects

1. The Docker image does not build. The base image lacks the CA bundle needed by
   the `curl`-based uv installer.
2. `uv pip install --system multicorn` resolves the unrelated PyPI package
   named `multicorn` (an experimental Python multi-interpreter server), not the
   PostgreSQL Multicorn extension.
3. The image installs `postgresql-server-dev-all`, which pulled development
   packages for PostgreSQL 10 through 18 and more than 1 GiB of build
   dependencies in the baseline probe.
4. The Docker integration test is skipped unless three environment variables
   are supplied, so normal local and CI runs never start PostgreSQL.
5. Even when enabled, that test sends a literal `{self.openapi_columns}` to
   PostgreSQL because the `CREATE FOREIGN TABLE` string is not an f-string.
6. CI only runs Python mocks. It never builds the image, creates the extension,
   creates a foreign table, or proves that a SQL query caused an HTTP request.

## Functional gaps

- `IMPORT FOREIGN SCHEMA` is not implemented. The project therefore cannot
  create foreign tables from an OpenAPI document.
- PostgreSQL columns are never inferred or created. The response schema is only
  used as a list of names after the user has already declared the table.
- OpenAPI `$ref`, response-level references, schema composition, server
  variables, YAML documents, nullable types, and OpenAPI 3.1 type arrays are not
  resolved.
- Query quals and `LIMIT` are ignored. There is no path-parameter, query-filter,
  sort, or limit pushdown.
- There is no pagination, retry policy, `Retry-After` handling, connection
  pooling, response-size limit, or response content-type validation.
- Authentication is limited to a static JSON header object. There is no API-key
  placement policy, bearer-token handling, user mapping, Vault integration, or
  per-request credential mechanism.
- Only array responses work. Common envelopes (`data`, `results`, `items`),
  single-object endpoints, GeoJSON, and POST-for-read endpoints are unsupported
  without brittle manual configuration or are impossible.
- Nested values are handed to PostgreSQL without a deliberate JSONB contract.
  There is no catch-all JSONB column for fields added by an API.
- HTTP errors discard the useful status/body context and replace it with the
  generic message `HTTP request failed`.
- `requests.request` creates no persistent session, so repeated scans cannot
  reuse connections.
- Packaging metadata is duplicated between `setup.py` and `pyproject.toml`, and
  the project description still says the wrapper is specifically for
  BrasilAPI.
- `AGENTS.md` still describes the old CoinCap package and paths that no longer
  exist.

## Baseline evidence

`uv run python -m unittest discover -s tests -v` reported ten passing tests and
one skipped Docker test. The passing tests completed in 0.013 seconds because
they exercise mocks and a small in-process HTTP server, not PostgreSQL.

The advertised Docker build failed before installing the project:

```text
curl: (77) error setting certificate file: /etc/ssl/certs/ca-certificates.crt
mv: cannot stat '/root/.local/bin/uv': No such file or directory
```

By contrast, a clean PostgreSQL 18 container with Supabase Wrappers 0.6.2 and
the OpenAPI Wasm wrapper 0.2.1 successfully queried live PokéAPI, NWS, and
BrasilAPI endpoints. That probe establishes that the product idea is viable,
while also exposing a documented 170-180 ms fixed Wasm initialization cost and
imperfect schema inference for response envelopes. Those results motivate the
native Rust design in [ARCHITECTURE.md](ARCHITECTURE.md).
