CREATE FOREIGN DATA WRAPPER openapi_fdw
  HANDLER open_api_fdw_handler
  VALIDATOR open_api_fdw_validator;

COMMENT ON FOREIGN DATA WRAPPER openapi_fdw IS
  'Read-only native FDW for JSON HTTP APIs with optional OpenAPI schema import';
