# Writable HTTP tables

Writes are explicit capabilities on a foreign table. A normal imported or
manual table has no mutation endpoints and remains read-only. Configuring one
operation does not implicitly enable the others.

## Table options

| Option | Meaning |
| --- | --- |
| `rowid_column` | PostgreSQL column holding the stable remote identity. Required by the callback adapter for any modification, including INSERT. |
| `rowid_parameter` | Name inside mutation path templates. Defaults to `rowid_column`. |
| `insert_endpoint` | Relative collection or item path used by INSERT. |
| `insert_method` | `POST` by default; `PUT` is also accepted. |
| `update_endpoint` | Relative item path used by UPDATE. It must contain the row identity placeholder. |
| `update_method` | `PATCH` by default; `PUT` is also accepted. |
| `delete_endpoint` | Relative item path used by DELETE. It must contain the row identity placeholder. |
| `write_columns` | Optional JSON array of PostgreSQL column names allowed in request bodies. |
| `write_mode` | `columns` (default), `attrs`, or `merge`. |

Mutation endpoints are always relative to the foreign server's `base_url` (or
the server URL resolved from its OpenAPI document). Absolute per-table URLs are
rejected so configured credentials cannot be redirected to another origin. Row
identity values are encoded as one URL path segment before substitution.

`delete_method` is accepted for declarative tooling but currently must be
`DELETE`.

## Request bodies

In `columns` mode, the FDW turns PostgreSQL values into JSON and applies
`column_map` to obtain API property names. The row identity and the catch-all
JSONB column are never included. If `write_columns` is absent, every other
typed column present in the modification row is eligible. SQL NULL becomes JSON
null. For UPDATE, PostgreSQL supplies the columns named by `SET`, so PATCH bodies
are naturally partial. A PUT statement must set every field required by the
remote replacement contract.

In `attrs` mode, the configured `attrs_column` must contain a non-NULL JSONB
object and becomes the complete body. This is the schema-flexible path for APIs
whose write contract evolves faster than table DDL.

In `merge` mode, the body starts with the `attrs` object and selected typed
columns overwrite matching properties. This allows convenient typed fields and
still preserves API-specific keys.

For a mapping such as:

```sql
column_map '{"display_name":"displayName","color":"/metadata/color"}'
```

the write body contains `displayName` and a nested `metadata.color` property.
Write-side JSON Pointers create objects; array mutation through a pointer is not
inferred.

## SQL and HTTP mapping

| SQL | HTTP | Automatic retries |
| --- | --- | --- |
| `INSERT` | POST or PUT | PUT only |
| `UPDATE` | PATCH or PUT | PUT only |
| `DELETE` | DELETE | Yes |

GET, HEAD, OPTIONS, TRACE, PUT, and DELETE have idempotent HTTP semantics and
use the configured bounded retry policy. POST and PATCH make only one attempt.
An ambiguous connection failure can still mean that the origin accepted a
request whose response was lost; applications should use API-supported
idempotency keys where available.

## Transaction and executor boundaries

PostgreSQL cannot enlist an arbitrary HTTP service in its transaction protocol.
Consequently:

- rolling back SQL does not compensate an accepted remote mutation;
- a multi-row statement sends one request at a time and can partially succeed;
- UPDATE sends the fields named in the SQL `SET` clause after applying the body
  whitelist;
- `RETURNING` is not currently supported by the callback adapter; and
- INSERT still needs a declared `rowid_column`, even when the remote service
  generates the value.

Use narrow predicates, explicit body whitelists, API idempotency facilities,
and single-row statements for business-critical side effects. The deterministic
PostgreSQL 14-18 suite exercises POST, PATCH, PUT, DELETE, dynamic JSONB bodies,
204 responses, body whitelisting, and the no-retry rule for POST.
