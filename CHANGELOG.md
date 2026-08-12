# Changelog

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
