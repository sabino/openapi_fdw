CREATE OR REPLACE FUNCTION "open_api_fdw_handler"()
RETURNS fdw_handler
STRICT
LANGUAGE c
AS 'openapi_fdw-0.4.1', 'open_api_fdw_handler_wrapper';

CREATE OR REPLACE FUNCTION "open_api_fdw_meta"()
RETURNS TABLE (
  "name" text,
  "version" text,
  "author" text,
  "website" text
)
STRICT
LANGUAGE c
AS 'openapi_fdw-0.4.1', 'open_api_fdw_meta_wrapper';

CREATE OR REPLACE FUNCTION "open_api_fdw_validator"(
  "options" text[],
  "catalog" oid
)
RETURNS void
LANGUAGE c
AS 'openapi_fdw-0.4.1', 'open_api_fdw_validator_wrapper';

COMMENT ON FOREIGN DATA WRAPPER openapi_fdw IS
  'Native FDW for JSON HTTP APIs with OpenAPI import and opt-in writes';
