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

DO $test$
DECLARE
  actual_ids bigint[];
  actual_id bigint;
  actual_name text;
  actual_color text;
  actual_future text;
  actual_tags text[];
  actual_timestamp timestamptz;
BEGIN
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
     WHERE attrs ->> 'path' = '/api/items'
       AND attrs #>> '{query,limit,0}' = '1'
  ) THEN
    RAISE EXCEPTION 'SQL LIMIT was not sent as an API limit';
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
     WHERE attrs ->> 'method' = 'POST'
       AND attrs #>> '{body,term}' = 'postgres'
  ) THEN
    RAISE EXCEPTION 'POST request body was not transmitted';
  END IF;
END
$test$;

\set QUIET off
SELECT 'native PG18 integration passed' AS result;
