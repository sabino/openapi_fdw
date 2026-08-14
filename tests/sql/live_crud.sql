\set ON_ERROR_STOP on
\set QUIET on

\if :{?spec_url}
\else
  \set spec_url https://raw.githubusercontent.com/sabino/openapi_fdw/main/examples/gorest.openapi.yaml
\endif
\if :{?marker}
\else
  \warn marker is required
  \quit 2
\endif
\if :{?email}
\else
  \warn email is required
  \quit 2
\endif

CREATE EXTENSION IF NOT EXISTS openapi_fdw;
CREATE SERVER live_crud_api
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    spec_url :'spec_url',
    bearer_token 'openapi-fdw-public-validation',
    max_retries '0',
    max_pages '2',
    request_timeout_ms '30000'
  );
CREATE SCHEMA live_crud;
IMPORT FOREIGN SCHEMA api
  LIMIT TO (list_users)
  FROM SERVER live_crud_api
  INTO live_crud
  OPTIONS (methods 'GET', include_attrs 'true', writable 'true');

SELECT set_config('openapi_fdw.live_marker', :'marker', false);
SELECT set_config('openapi_fdw.live_email', :'email', false);

DO $test$
DECLARE
  expected_marker text := current_setting('openapi_fdw.live_marker');
  expected_email text := current_setting('openapi_fdw.live_email');
  created_id bigint;
  actual_status text;
BEGIN
  INSERT INTO live_crud.list_users (name, email, gender, status)
  VALUES (expected_marker, expected_email, 'male', 'active');

  SELECT id, status INTO STRICT created_id, actual_status
    FROM live_crud.list_users
   WHERE email = expected_email;
  IF created_id IS NULL OR actual_status <> 'active' THEN
    RAISE EXCEPTION 'public INSERT/GET mismatch: %, %', created_id, actual_status;
  END IF;

  UPDATE live_crud.list_users
     SET status = 'inactive'
   WHERE email = expected_email;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'public user % was not patched', expected_email;
  END IF;

  SELECT status INTO STRICT actual_status
    FROM live_crud.list_users
   WHERE email = expected_email;
  IF actual_status <> 'inactive' THEN
    RAISE EXCEPTION 'public PATCH/GET mismatch: %', actual_status;
  END IF;
END
$test$;

ALTER FOREIGN TABLE live_crud.list_users OPTIONS (SET update_method 'PUT');

DO $test$
DECLARE
  expected_marker text := current_setting('openapi_fdw.live_marker');
  expected_email text := current_setting('openapi_fdw.live_email');
  actual_name text;
  actual_gender text;
  actual_status text;
BEGIN
  UPDATE live_crud.list_users
     SET name = expected_marker || ' replaced',
         email = expected_email,
         gender = 'female',
         status = 'active'
   WHERE email = expected_email;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'public user % was not replaced', expected_email;
  END IF;

  SELECT name, gender, status
    INTO STRICT actual_name, actual_gender, actual_status
    FROM live_crud.list_users
   WHERE email = expected_email;
  IF actual_name <> (expected_marker || ' replaced')
     OR actual_gender <> 'female'
     OR actual_status <> 'active' THEN
    RAISE EXCEPTION 'public PUT/GET mismatch: %, %, %',
      actual_name, actual_gender, actual_status;
  END IF;

  DELETE FROM live_crud.list_users WHERE email = expected_email;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'public user % was not deleted', expected_email;
  END IF;
  IF EXISTS (
    SELECT 1 FROM live_crud.list_users WHERE email = expected_email
  ) THEN
    RAISE EXCEPTION 'public DELETE left user % behind', expected_email;
  END IF;
END
$test$;

\set QUIET off
SELECT 'public OpenAPI INSERT/GET/PATCH/PUT/DELETE passed with cleanup' AS result;
