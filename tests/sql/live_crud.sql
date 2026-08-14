\set ON_ERROR_STOP on
\set QUIET on

\if :{?spec_url}
\else
  \set spec_url https://raw.githubusercontent.com/sabino/openapi_fdw/main/examples/restful-api.openapi.yaml
\endif
\if :{?object_id}
\else
  \warn object_id is required
  \quit 2
\endif

CREATE EXTENSION IF NOT EXISTS openapi_fdw;
CREATE SERVER live_crud_api
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    spec_url :'spec_url',
    max_retries '0',
    max_pages '2',
    request_timeout_ms '30000'
  );
CREATE SCHEMA live_crud;
IMPORT FOREIGN SCHEMA api
  LIMIT TO (get_object)
  FROM SERVER live_crud_api
  INTO live_crud
  OPTIONS (methods 'GET', include_attrs 'true', writable 'true');

SELECT set_config('openapi_fdw.live_object_id', :'object_id', false);

DO $test$
DECLARE
  expected_id text := current_setting('openapi_fdw.live_object_id');
  actual_name text;
  actual_data jsonb;
BEGIN
  UPDATE live_crud.get_object
     SET name = 'openapi_fdw public CRUD validation',
         data = '{"stage":"updated-from-postgresql"}'::jsonb
   WHERE id = expected_id;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'public object % was not updated', expected_id;
  END IF;

  SELECT name, data INTO STRICT actual_name, actual_data
    FROM live_crud.get_object
   WHERE id = expected_id;
  IF actual_name <> 'openapi_fdw public CRUD validation'
     OR actual_data <> '{"stage":"updated-from-postgresql"}'::jsonb THEN
    RAISE EXCEPTION 'public PATCH/GET mismatch: %, %', actual_name, actual_data;
  END IF;

  DELETE FROM live_crud.get_object WHERE id = expected_id;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'public object % was not deleted', expected_id;
  END IF;
END
$test$;

\set QUIET off
SELECT 'public OpenAPI PATCH/GET/DELETE passed with cleanup' AS result;
