# Migration from the Hy/Multicorn prototype

The native extension is a replacement, not an in-place Python package upgrade.
Existing Multicorn servers and tables keep referring to the old wrapper until
their DDL is migrated.

## Option mapping

| Prototype option | Native location | Native option |
| --- | --- | --- |
| `openapi_url` | foreign server | `spec_url` |
| `server_url` | foreign server | `base_url` |
| `headers` | foreign server | `headers` |
| `timeout` in seconds | foreign server | `request_timeout_ms` |
| `path` | foreign table | `endpoint` |
| `method` | foreign table | `method` |
| `data_path` | foreign table | `response_path` as an RFC 6901 pointer |
| `query_params` | foreign table | `query_params` |

Authentication, retries, pagination, size limits, path/query pushdown, and the
JSONB catch-all did not have equivalent prototype options.

## Side-by-side migration

Install `openapi_fdw`, keep Multicorn temporarily, and create a new server under
a different name:

```sql
CREATE EXTENSION openapi_fdw;

CREATE SERVER upstream_native
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    spec_url 'https://api.example/openapi.json',
    base_url 'https://api.example/v1'
  );

CREATE FOREIGN TABLE items_native (
  id bigint,
  name text,
  attrs jsonb
)
SERVER upstream_native
OPTIONS (
  endpoint '/items',
  response_path '/results'
);
```

Compare representative queries, null/type behavior, pagination totals, and the
API's request logs. Then change dependent views or rename tables in one
transaction. The two extensions can coexist during the comparison.

For a large OpenAPI service, import into a staging schema first:

```sql
CREATE SCHEMA api_next;
IMPORT FOREIGN SCHEMA api
  FROM SERVER upstream_native
  INTO api_next
  OPTIONS (methods 'GET', include_attrs 'true');
```

Review the generated types before exposing that schema to applications.

## Behavioral differences

- The native FDW supports arrays, objects, common envelopes, single objects,
  and GeoJSON rather than requiring a root JSON array.
- Imported primitive arrays use PostgreSQL arrays; nested/heterogeneous values
  use JSONB.
- Type mismatches fail the query by default. Set `on_type_error 'null'` on a
  table only when lossy behavior is intentional.
- HTTPS is required unless `allow_http 'true'` is explicitly set for a trusted
  local service.
- Redirects and pagination cannot cross origins by default.
- API errors include bounded status/body context, with configured credentials
  redacted.
- A scan is always live. Neither implementation supplies a result cache.

## Removing the prototype

After all dependencies have moved, drop the old foreign tables and servers,
then drop Multicorn only if no unrelated wrapper still uses it:

```sql
DROP SERVER old_openapi_server CASCADE;
-- Check other Multicorn servers before running this:
-- DROP EXTENSION multicorn;
```

The Hy sources remain recoverable from repository history; no runtime migration
requires retaining Python or Hy in the database image.
