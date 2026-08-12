#!/bin/sh
set -eu

repository=${OPENAPI_FDW_REPOSITORY:-sabino/openapi_fdw}
version=${OPENAPI_FDW_VERSION:-}
pg_config=${PG_CONFIG:-pg_config}

usage() {
  printf '%s\n' \
    'Usage: install.sh [--version v0.3.2] [--pg-config /path/to/pg_config]' \
    '' \
    'Installs a checksummed glibc/Linux x86-64 OpenAPI FDW release for the' \
    'PostgreSQL major reported by pg_config.'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      version=${2:?--version requires a value}
      shift 2
      ;;
    --pg-config)
      pg_config=${2:?--pg-config requires a value}
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

for command in curl find install mkdir mktemp sed sha256sum tar uname; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'Required command is missing: %s\n' "$command" >&2
    exit 1
  }
done
command -v "$pg_config" >/dev/null 2>&1 || {
  printf 'pg_config was not found: %s\n' "$pg_config" >&2
  exit 1
}

if [ -z "$version" ]; then
  latest_url=$(curl --fail --silent --show-error --location \
    --output /dev/null --write-out '%{url_effective}' \
    "https://github.com/$repository/releases/latest")
  version=${latest_url##*/}
fi

case "$version" in
  v[0-9]* ) ;;
  *)
    printf 'Release version must start with v and a digit: %s\n' "$version" >&2
    exit 1
    ;;
esac
case "$version" in
  *[!A-Za-z0-9._-]* )
    printf 'Release version contains unsupported characters: %s\n' "$version" >&2
    exit 1
    ;;
esac

architecture=$(uname -m)
test "$architecture" = x86_64 || {
  printf 'Prebuilt native packages currently support x86_64, got %s. Use cargo pgrx install on this host.\n' "$architecture" >&2
  exit 1
}

pg_major=$($pg_config --version | sed -E 's/^[^0-9]*([0-9]+).*/\1/')
case "$pg_major" in
  14|15|16|17|18) ;;
  *)
    printf 'Unsupported PostgreSQL major from pg_config: %s\n' "$pg_major" >&2
    exit 1
    ;;
esac

archive="openapi_fdw-${version}-pg${pg_major}-linux-amd64.tar.gz"
base_url="https://github.com/$repository/releases/download/$version"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/openapi-fdw-install.XXXXXX")
trap 'find "$temporary" -xdev -depth -delete' EXIT HUP INT TERM

curl --fail --silent --show-error --location \
  --output "$temporary/$archive" "$base_url/$archive"
curl --fail --silent --show-error --location \
  --output "$temporary/$archive.sha256" "$base_url/$archive.sha256"
(cd "$temporary" && sha256sum --check "$archive.sha256")
mkdir "$temporary/package"
tar -C "$temporary/package" -xzf "$temporary/$archive"

library=$(find "$temporary/package" -maxdepth 1 -type f \
  -name 'openapi_fdw-*.so' -print -quit)
control="$temporary/package/openapi_fdw.control"
sql_files=$(find "$temporary/package" -maxdepth 1 -type f -name 'openapi_fdw--*.sql' -print)
test -n "$library" && test -f "$library" && test -f "$control" && test -n "$sql_files" || {
  printf 'Release archive is incomplete.\n' >&2
  exit 1
}

pkglibdir=$($pg_config --pkglibdir)
extension_dir=$($pg_config --sharedir)/extension
install -d "$pkglibdir" "$extension_dir"
install -m 0755 "$library" "$pkglibdir/${library##*/}"
install -m 0644 "$control" "$extension_dir/openapi_fdw.control"
for sql_file in $sql_files; do
  install -m 0644 "$sql_file" "$extension_dir/${sql_file##*/}"
done

printf 'Installed OpenAPI FDW %s for PostgreSQL %s. Run CREATE EXTENSION for a new database or ALTER EXTENSION openapi_fdw UPDATE for an existing installation.\n' \
  "$version" "$pg_major"
