CREATE FOREIGN DATA WRAPPER openapi_fdw
  HANDLER open_api_fdw_handler
  VALIDATOR open_api_fdw_validator;

COMMENT ON FOREIGN DATA WRAPPER openapi_fdw IS
  'Native FDW for JSON HTTP APIs with OpenAPI import and opt-in writes';
