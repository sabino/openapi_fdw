# Changelog

## 0.4.1 - 2026-08-14

- Keep writable discovery dialogs within narrow viewports and show every
  inferred write capability without horizontal page scrolling.
- Ship the `0.4.0` to `0.4.1` extension upgrade script so patch releases stay
  uniform across container images and native packages.

## 0.4.0 - 2026-08-14

- Add explicit native SQL INSERT/UPDATE/DELETE mapping to POST/PUT/PATCH/DELETE
  endpoints, with typed body whitelists and schema-flexible JSONB modes.
- Infer safe collection/item CRUD pairs and request-body columns when
  `IMPORT FOREIGN SCHEMA` opts into `writable 'true'`.
- Never retry non-idempotent POST/PATCH mutations automatically and document
  the HTTP transaction, multi-row, and `RETURNING` boundaries.
- Add control-plane writable discovery, portable bundle support, deterministic
  PostgreSQL 14-18 CRUD coverage, a public CRUD adapter contract, and a manual
  disposable-object live validation workflow.
- Ship the `0.3.2` to `0.4.0` extension upgrade script in images and native
  packages.

## 0.3.2 - 2026-08-12

- Treat path-template tables without all required equality predicates as empty
  parameterized lookups without making an HTTP request. This lets PostgreSQL
  catalog consumers and BI schema synchronization register the declared fields
  while preserving live requests for bound lookups.
- Ship and install the `0.3.1` to `0.3.2` extension upgrade script alongside
  the initial SQL file in container and native packages.

## 0.3.1 - 2026-08-12

- Preserve pgrx's versioned shared-library filename in native release archives
  and installations, and test the archive contract before publishing a tag.

## 0.3.0 - 2026-08-12

- Add a small, stateless Rust control plane with token authentication, OpenAPI
  discovery, selectable tables, redacted SQL previews, transactional apply and
  replace, live-row previews, and explicit source removal.
- Add portable, redacted `openapi-fdw/v1` setup bundles with validation and
  transactional import.
- Resolve bearer tokens, API keys, and arbitrary headers from PostgreSQL
  process environment variables, while preserving literal options for simple
  deployments.
- Add a directly importable BrasilAPI definition and end-to-end control-plane
  integration coverage.
- Default release containers to Alpine, add a stripped scratch control-plane
  image, add checksummed native packages for PostgreSQL 14 through 18, and add
  an optional one-click deployment definition.

## 0.2.0 - 2026-08-12

- Replace the Hy/Python/Multicorn prototype with a native Rust PostgreSQL FDW.
- Support PostgreSQL 14 through 18.
- Add OpenAPI 3.0/3.1 JSON/YAML schema import with a default JSONB catch-all.
- Add pooled HTTPS, authentication, retries, response bounds, pagination, path
  and query parameters, safe LIMIT behavior, typed coercion, and GeoJSON.
- Add deterministic PostgreSQL integration, live public API smoke tests,
  benchmarks, per-major CI, and release container workflows.

## 0.1.0

Historical experimental Hy/Multicorn implementation. It was never validated as
a working PostgreSQL extension and remains available in Git history only.
