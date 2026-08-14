\set ON_ERROR_STOP on
\set QUIET on

\if :{?spec_url}
\else
  \set spec_url http://127.0.0.1:18080/openapi.json
\endif
\if :{?api_base_url}
\else
  \set api_base_url http://127.0.0.1:18080/api
\endif

CREATE EXTENSION IF NOT EXISTS openapi_fdw;

CREATE SERVER mock_api
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    spec_url :'spec_url',
    allow_http 'true',
    headers '{"x-test-header":"integration"}',
    api_key 'integration-placeholder',
    max_retries '2',
    max_pages '10'
  );

CREATE SCHEMA imported;
IMPORT FOREIGN SCHEMA api
  FROM SERVER mock_api
  INTO imported
  OPTIONS (methods 'GET', include_attrs 'true');

CREATE SCHEMA imported_writable;
IMPORT FOREIGN SCHEMA api
  LIMIT TO (list_writable_items)
  FROM SERVER mock_api
  INTO imported_writable
  OPTIONS (methods 'GET', include_attrs 'true', writable 'true');

CREATE SERVER environment_auth
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url :'api_base_url',
    allow_http 'true',
    headers_env '{"x-env-header":"OPENAPI_FDW_TEST_HEADER"}',
    api_key_env 'OPENAPI_FDW_TEST_API_KEY',
    api_key_name 'x-env-api-key',
    max_pages '2'
  );
CREATE FOREIGN TABLE environment_auth_items (attrs jsonb)
  SERVER environment_auth OPTIONS (endpoint '/items', pagination 'none');
SELECT attrs FROM environment_auth_items LIMIT 1;

DO $test$
DECLARE
  created_id text;
  actual_name text;
BEGIN
  IF NOT EXISTS (
    SELECT 1
      FROM pg_foreign_table AS foreign_table,
           LATERAL pg_options_to_table(foreign_table.ftoptions) AS option
     WHERE foreign_table.ftrelid = 'imported_writable.list_writable_items'::regclass
       AND option.option_name = 'update_method'
       AND option.option_value = 'PATCH'
  ) THEN
    RAISE EXCEPTION 'writable OpenAPI import did not infer PATCH';
  END IF;

  INSERT INTO imported_writable.list_writable_items (name, data)
  VALUES ('Imported through OpenAPI', '{"source":"import"}'::jsonb);
  SELECT id INTO STRICT created_id
    FROM imported_writable.list_writable_items
   WHERE name = 'Imported through OpenAPI';
  UPDATE imported_writable.list_writable_items
     SET name = 'Updated through OpenAPI'
   WHERE id = created_id;
  SELECT name INTO STRICT actual_name
    FROM imported_writable.list_writable_items
   WHERE id = created_id;
  IF actual_name <> 'Updated through OpenAPI' THEN
    RAISE EXCEPTION 'inferred writable table did not update the API: %', actual_name;
  END IF;
  DELETE FROM imported_writable.list_writable_items WHERE id = created_id;
END
$test$;

DO $test$
DECLARE
  actual_ids bigint[];
  actual_id bigint;
  actual_count bigint;
  actual_name text;
  actual_color text;
  actual_future text;
  actual_tags text[];
  actual_timestamp timestamptz;
BEGIN
  SELECT count(*)
    INTO actual_count
    FROM imported.get_by_slug;
  IF actual_count <> 0 THEN
    RAISE EXCEPTION 'unbound path lookup should be empty, got % rows', actual_count;
  END IF;

  SELECT array_agg(id ORDER BY id)
    INTO actual_ids
    FROM imported.list_items;
  IF actual_ids <> ARRAY[1, 2, 3]::bigint[] THEN
    RAISE EXCEPTION 'pagination or typed import failed: %', actual_ids;
  END IF;

  SELECT id INTO STRICT actual_id
    FROM imported.list_items
   ORDER BY id DESC
   LIMIT 1;
  IF actual_id <> 3 THEN
    RAISE EXCEPTION 'local ORDER BY/LIMIT was incorrectly bounded remotely: %', actual_id;
  END IF;

  SELECT id INTO STRICT actual_id
    FROM imported.list_items
   LIMIT 1 OFFSET 1;
  IF actual_id <> 2 THEN
    RAISE EXCEPTION 'LIMIT/OFFSET contract failed: %', actual_id;
  END IF;

  SELECT display_name,
         attrs #>> '{meta,color}',
         attrs ->> 'futureField',
         tags,
         created_at
    INTO actual_name, actual_color, actual_future, actual_tags, actual_timestamp
    FROM imported.list_items
   WHERE id = 2;
  IF actual_name <> 'Beta'
     OR actual_color <> 'blue'
     OR actual_future <> 'also kept'
     OR actual_tags <> ARRAY['two']::text[]
     OR actual_timestamp <> '2026-08-11 11:30:00+00'::timestamptz THEN
    RAISE EXCEPTION 'typed/JSONB projection failed: %, %, %, %, %',
      actual_name, actual_color, actual_future, actual_tags, actual_timestamp;
  END IF;

  SELECT name
    INTO actual_name
    FROM imported.get_by_slug
   WHERE slug = 'mr mime';
  IF actual_name <> 'resolved:mr mime' THEN
    RAISE EXCEPTION 'path substitution failed: %', actual_name;
  END IF;

  SELECT name
    INTO actual_name
    FROM imported.list_stations
   WHERE station_identifier = 'KSEA';
  IF actual_name <> 'Seattle' THEN
    RAISE EXCEPTION 'GeoJSON projection failed: %', actual_name;
  END IF;
END
$test$;

CREATE FOREIGN TABLE jsonb_only (attrs jsonb)
  SERVER mock_api
  OPTIONS (endpoint '/items');

DO $test$
DECLARE
  future_value text;
BEGIN
  SELECT attrs ->> 'futureField'
    INTO future_value
    FROM jsonb_only
   WHERE attrs ->> 'id' = '1';
  IF future_value <> 'kept without DDL changes' THEN
    RAISE EXCEPTION 'JSONB-only table lost an undeclared field: %', future_value;
  END IF;
END
$test$;

CREATE FOREIGN TABLE post_search (
  term text,
  found boolean,
  attrs jsonb
)
SERVER mock_api
OPTIONS (
  endpoint '/search',
  method 'POST',
  request_body '{"term":"postgres"}',
  response_path '/results'
);

DO $test$
DECLARE
  result text;
BEGIN
  SELECT search.term || ':' || search.found::text
    INTO result
    FROM post_search AS search;
  IF result <> 'postgres:true' THEN
    RAISE EXCEPTION 'POST scan failed: %', result;
  END IF;
END
$test$;

CREATE FOREIGN TABLE writable_items (
  id text,
  name text,
  data jsonb,
  server_only text,
  attrs jsonb
)
SERVER mock_api
OPTIONS (
  endpoint '/writable-items',
  pagination 'none',
  column_map '{"server_only":"serverOnly"}',
  rowid_column 'id',
  rowid_parameter 'itemId',
  insert_endpoint '/writable-items',
  update_endpoint '/writable-items/{itemId}',
  delete_endpoint '/writable-items/{itemId}',
  write_columns '["name","data"]'
);

DO $test$
DECLARE
  created_id text;
  actual_name text;
  actual_data jsonb;
BEGIN
  INSERT INTO writable_items (name, data)
  VALUES ('Created through SQL', '{"stage":"insert"}'::jsonb);

  SELECT id INTO STRICT created_id
    FROM writable_items
   WHERE name = 'Created through SQL';
  IF created_id !~ '^generated-[0-9]+$' THEN
    RAISE EXCEPTION 'POST did not create a remotely generated identity: %', created_id;
  END IF;

  UPDATE writable_items
     SET name = 'Patched through SQL', data = '{"stage":"patch"}'::jsonb
   WHERE id = created_id;
  SELECT name, data INTO STRICT actual_name, actual_data
    FROM writable_items
   WHERE id = created_id;
  IF actual_name <> 'Patched through SQL'
     OR actual_data <> '{"stage":"patch"}'::jsonb THEN
    RAISE EXCEPTION 'PATCH did not persist through SQL: %, %', actual_name, actual_data;
  END IF;
END
$test$;

ALTER FOREIGN TABLE writable_items OPTIONS (ADD update_method 'PUT');

DO $test$
DECLARE
  created_id text;
  actual_name text;
  actual_data jsonb;
BEGIN
  SELECT id INTO STRICT created_id
    FROM writable_items
   WHERE name = 'Patched through SQL';
  UPDATE writable_items
     SET name = 'Replaced through SQL', data = '{"stage":"put"}'::jsonb
   WHERE id = created_id;
  SELECT name, data INTO STRICT actual_name, actual_data
    FROM writable_items
   WHERE id = created_id;
  IF actual_name <> 'Replaced through SQL'
     OR actual_data <> '{"stage":"put"}'::jsonb THEN
    RAISE EXCEPTION 'PUT did not persist through SQL: %, %', actual_name, actual_data;
  END IF;

  DELETE FROM writable_items WHERE id = created_id;
  IF EXISTS (SELECT 1 FROM writable_items WHERE id = created_id) THEN
    RAISE EXCEPTION 'DELETE did not remove the remote row';
  END IF;
END
$test$;

CREATE FOREIGN TABLE writable_attrs (
  id text,
  name text,
  data jsonb,
  attrs jsonb
)
SERVER mock_api
OPTIONS (
  endpoint '/writable-items',
  pagination 'none',
  rowid_column 'id',
  rowid_parameter 'itemId',
  insert_endpoint '/writable-items',
  delete_endpoint '/writable-items/{itemId}',
  write_mode 'attrs'
);

DO $test$
DECLARE
  created_id text;
  actual_data jsonb;
BEGIN
  INSERT INTO writable_attrs (attrs)
  VALUES ('{"name":"JSONB through SQL","data":{"dynamic":true},"newApiField":42}'::jsonb);
  SELECT id, data INTO STRICT created_id, actual_data
    FROM writable_attrs
   WHERE name = 'JSONB through SQL';
  IF actual_data <> '{"dynamic":true}'::jsonb THEN
    RAISE EXCEPTION 'JSONB write body was not preserved: %', actual_data;
  END IF;
  DELETE FROM writable_attrs WHERE id = created_id;
END
$test$;

CREATE FOREIGN TABLE non_retrying_write (
  id text,
  name text
)
SERVER mock_api
OPTIONS (
  endpoint '/writable-items',
  pagination 'none',
  rowid_column 'id',
  insert_endpoint '/flaky-write',
  write_columns '["name"]'
);
DO $test$
BEGIN
  BEGIN
    INSERT INTO non_retrying_write (name) VALUES ('must run once');
    RAISE EXCEPTION 'failing POST mutation unexpectedly succeeded';
  EXCEPTION
    WHEN SQLSTATE 'HV00L' THEN NULL;
  END;
END
$test$;

CREATE FOREIGN TABLE flaky (id bigint)
  SERVER mock_api OPTIONS (endpoint '/flaky');
DO $test$
DECLARE
  actual bigint;
BEGIN
  SELECT id INTO actual FROM flaky;
  IF actual <> 99 THEN
    RAISE EXCEPTION 'retry handling failed: %', actual;
  END IF;
END
$test$;

CREATE FOREIGN TABLE nullable_bad_type (id bigint)
  SERVER mock_api
  OPTIONS (endpoint '/bad-type', on_type_error 'null');
DO $test$
BEGIN
  IF (SELECT id FROM nullable_bad_type) IS NOT NULL THEN
    RAISE EXCEPTION 'on_type_error=null did not return NULL';
  END IF;
END
$test$;

CREATE FOREIGN TABLE strict_bad_type (id bigint)
  SERVER mock_api OPTIONS (endpoint '/bad-type');
DO $test$
BEGIN
  BEGIN
    PERFORM id FROM strict_bad_type;
    RAISE EXCEPTION 'strict conversion unexpectedly succeeded';
  EXCEPTION
    WHEN SQLSTATE 'HV004' THEN NULL;
  END;
END
$test$;

CREATE FOREIGN TABLE wrong_content_type (id bigint)
  SERVER mock_api OPTIONS (endpoint '/wrong-content-type');
DO $test$
BEGIN
  BEGIN
    PERFORM id FROM wrong_content_type;
    RAISE EXCEPTION 'non-JSON Content-Type unexpectedly succeeded';
  EXCEPTION
    WHEN SQLSTATE 'HV004' THEN NULL;
  END;
END
$test$;

CREATE FOREIGN TABLE cross_origin (id bigint)
  SERVER mock_api OPTIONS (endpoint '/cross-origin');
DO $test$
BEGIN
  BEGIN
    PERFORM count(*) FROM cross_origin;
    RAISE EXCEPTION 'cross-origin pagination unexpectedly succeeded';
  EXCEPTION
    WHEN SQLSTATE 'HV004' THEN NULL;
  END;
END
$test$;

CREATE FOREIGN TABLE pagination_loop (id bigint)
  SERVER mock_api OPTIONS (endpoint '/loop');
DO $test$
BEGIN
  BEGIN
    PERFORM count(*) FROM pagination_loop;
    RAISE EXCEPTION 'duplicate pagination token unexpectedly succeeded';
  EXCEPTION
    WHEN SQLSTATE 'HV004' THEN NULL;
  END;
END
$test$;

CREATE SERVER tiny_response
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url :'api_base_url',
    allow_http 'true',
    max_response_bytes '128',
    max_retries '0'
  );
CREATE FOREIGN TABLE oversized (attrs jsonb)
  SERVER tiny_response OPTIONS (endpoint '/large');
DO $test$
BEGIN
  BEGIN
    PERFORM attrs FROM oversized;
    RAISE EXCEPTION 'oversized response unexpectedly succeeded';
  EXCEPTION
    WHEN SQLSTATE 'HV004' THEN NULL;
  END;
END
$test$;

-- Exercise SQL LIMIT without a sort/filter so it is safe to push to the API.
SELECT id FROM imported.list_items LIMIT 1;

CREATE FOREIGN TABLE request_log (attrs jsonb)
  SERVER mock_api
  OPTIONS (endpoint '/__requests', pagination 'none');

DO $test$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'path' = '/openapi.json'
       AND attrs ->> 'testHeader' IS NULL
       AND NOT (attrs ->> 'hasApiKey')::boolean
  ) THEN
    RAISE EXCEPTION 'API credentials were forwarded to the specification URL by default';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'path' = '/api/items'
       AND attrs #>> '{query,limit,0}' = '1'
  ) THEN
    RAISE EXCEPTION 'SQL LIMIT was not sent as an API limit';
  END IF;
  IF (SELECT count(*) FROM request_log
       WHERE attrs ->> 'path' LIKE '/api/by-slug/%') <> 1 THEN
    RAISE EXCEPTION 'an unbound path lookup made an HTTP request';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'testHeader' = 'integration'
       AND (attrs ->> 'hasApiKey')::boolean
  ) THEN
    RAISE EXCEPTION 'configured headers/authentication were not sent';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE (attrs ->> 'validEnvAuth')::boolean
  ) THEN
    RAISE EXCEPTION 'environment-backed headers/authentication were not sent';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'method' = 'POST'
       AND attrs #>> '{body,term}' = 'postgres'
  ) THEN
    RAISE EXCEPTION 'POST request body was not transmitted';
  END IF;
  IF (SELECT count(*) FROM request_log
       WHERE attrs ->> 'path' = '/api/flaky-write'
         AND attrs ->> 'method' = 'POST') <> 1 THEN
    RAISE EXCEPTION 'non-idempotent POST mutation was retried';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'method' = 'POST'
       AND attrs ->> 'path' = '/api/writable-items'
       AND attrs #>> '{body,name}' = 'Created through SQL'
       AND attrs #> '{body,data}' = '{"stage":"insert"}'::jsonb
       AND NOT (attrs -> 'body' ? 'id')
       AND NOT (attrs -> 'body' ? 'serverOnly')
  ) THEN
    RAISE EXCEPTION 'INSERT body whitelist was not respected';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'method' = 'PATCH'
       AND attrs ->> 'path' LIKE '/api/writable-items/generated%'
       AND attrs #>> '{body,name}' = 'Patched through SQL'
       AND attrs #> '{body,data}' = '{"stage":"patch"}'::jsonb
  ) THEN
    RAISE EXCEPTION 'UPDATE was not mapped to PATCH with the expected body';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'method' = 'PUT'
       AND attrs ->> 'path' LIKE '/api/writable-items/generated%'
       AND attrs #>> '{body,name}' = 'Replaced through SQL'
       AND attrs #> '{body,data}' = '{"stage":"put"}'::jsonb
  ) THEN
    RAISE EXCEPTION 'UPDATE was not mapped to PUT';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'method' = 'DELETE'
       AND attrs ->> 'path' LIKE '/api/writable-items/generated%'
  ) THEN
    RAISE EXCEPTION 'DELETE was not sent to the row identity endpoint';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM request_log
     WHERE attrs ->> 'method' = 'POST'
       AND attrs ->> 'path' = '/api/writable-items'
       AND attrs #>> '{body,name}' = 'JSONB through SQL'
       AND attrs #>> '{body,newApiField}' = '42'
  ) THEN
    RAISE EXCEPTION 'JSONB write mode did not forward dynamic API fields';
  END IF;
END
$test$;

\set QUIET off
SELECT 'native PG18 integration passed' AS result;
