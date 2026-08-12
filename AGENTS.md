# OpenAPI FDW contributor guidance

This repository implements a native, read-only PostgreSQL FDW for JSON HTTP
APIs. The runtime is Rust with pgrx and Supabase Wrappers. The former
Hy/Python/Multicorn implementation is historical and must not be reintroduced
as an in-database dependency.

## Important paths

- `src/fdw.rs`: PostgreSQL scan/import callbacks and JSON-to-cell conversion.
- `src/http.rs`: bounded pooled HTTP client, redirects, retries, and spec fetch.
- `src/spec.rs`: OpenAPI 3.0/3.1 import and safe SQL generation.
- `src/options.rs`: validated server/table options and credential redaction.
- `tests/mock_api.py`: deterministic real HTTP origin for integration tests.
- `tests/sql/native_integration.sql`: end-to-end PostgreSQL assertions.
- `Dockerfile`: per-PostgreSQL-major build and minimal runtime image.

## Validation

Use `cargo fmt --all -- --check` for the quick local check. A full validation
must build the extension for the target PostgreSQL major, run an actual
PostgreSQL server, and execute `tests/sql/native_integration.sql` against the
deterministic API. Mock-only Rust/Python results are not sufficient evidence.

The CI matrix covers PostgreSQL 14 through 18. Live public API checks are
separate from deterministic merge checks because external availability is not
under this project's control.

## Resource and safety constraints

Rust/pgrx release builds can create roughly 2 GiB of intermediates. On a
constrained workstation, use a bounded tmpfs target or let CI build the matrix.
Never broadly prune Docker images, build cache, or volumes; remove only exact
task-owned artifacts after confirming their identity.

Keep HTTPS verification enabled by default, preserve response/time/page caps,
never include credentials in errors or fixtures, and do not forward redirects
or pagination across origins without an explicit opt-in.
