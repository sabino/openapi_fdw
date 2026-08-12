#!/usr/bin/env sh
set -eu

control_origin=${CONTROL_ORIGIN:-http://127.0.0.1:18081}
admin_token=${OPENAPI_FDW_ADMIN_TOKEN:?OPENAPI_FDW_ADMIN_TOKEN is required}
spec_url=${SPEC_URL:-http://host.docker.internal:18080/openapi.json}

api() {
  method=$1
  path=$2
  body=${3-}
  if [ -n "$body" ]; then
    curl --fail --silent --show-error \
      --request "$method" \
      --header "Authorization: Bearer $admin_token" \
      --header "X-OpenAPI-FDW-Request: control-plane" \
      --header "Content-Type: application/json" \
      --data "$body" \
      "$control_origin$path"
  else
    curl --fail --silent --show-error \
      --request "$method" \
      --header "Authorization: Bearer $admin_token" \
      "$control_origin$path"
  fi
}

source_definition=$(jq --null-input \
  --arg spec "$spec_url" \
  '{
    name: "integration_api",
    schema: "integration_control",
    remoteSchema: "api",
    specUrl: $spec,
    methods: ["GET"],
    includeAttrs: true,
    tables: [],
    auth: {type: "none"},
    settings: {allowHttp: true}
  }')

curl --fail --silent --show-error "$control_origin/healthz" \
  | jq --exit-status '.status == "ok"' >/dev/null

unauthorized=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  "$control_origin/api/v1/state")
test "$unauthorized" = "401"

api GET /api/v1/state \
  | jq --exit-status '.sources == [] and (.postgresVersion | length > 0)' >/dev/null

discovery=$(api POST /api/v1/discover "$source_definition")
printf '%s' "$discovery" \
  | jq --exit-status '.tables | map(.name) | index("list_items") != null' >/dev/null

selected=$(printf '%s' "$source_definition" \
  | jq '.tables = ["list_items"]')
request=$(jq --null-input --argjson source "$selected" '{source: $source, replace: false}')

plan=$(api POST /api/v1/sources/plan "$request")
printf '%s' "$plan" \
  | jq --exit-status '.sql | contains("IMPORT FOREIGN SCHEMA")' >/dev/null

api POST /api/v1/sources "$request" \
  | jq --exit-status '.ok and (.sql | contains("list_items"))' >/dev/null

api GET /api/v1/state \
  | jq --exit-status '
      .sources[0].name == "integration_api"
      and .sources[0].managed
      and (.sources[0].tables | map(.name) | index("list_items") != null)' >/dev/null

api GET '/api/v1/sources/integration_api/tables/integration_control/list_items/rows?limit=2' \
  | jq --exit-status '.rows | length == 2 and .[0].attrs.futureField != null' >/dev/null

api GET /api/v1/export \
  | jq --exit-status '.apiVersion == "openapi-fdw/v1" and .sources[0].name == "integration_api"' >/dev/null

api DELETE /api/v1/sources/integration_api '{"confirm":"integration_api"}' \
  | jq --exit-status '.ok' >/dev/null

api GET /api/v1/state \
  | jq --exit-status '.sources == []' >/dev/null

printf '%s\n' 'control-plane integration passed'
