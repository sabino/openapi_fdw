#!/bin/sh
set -eu

repository=${OPENAPI_FDW_REPOSITORY:-sabino/openapi_fdw}
version=${OPENAPI_FDW_VERSION:-}
pg_config=${PG_CONFIG:-pg_config}

usage() {
  printf '%s\n' \
    'Usage: install.sh [--version v0.3.0] [--pg-config /path/to/pg_config]' \
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

for command in curl sha256sum tar install mktemp; do
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

library="$temporary/package/openapi_fdw.so"
control="$temporary/package/openapi_fdw.control"
sql=$(find "$temporary/package" -maxdepth 1 -type f -name 'openapi_fdw--*.sql' -print -quit)
test -f "$library" && test -f "$control" && test -n "$sql" || {
  printf 'Release archive is incomplete.\n' >&2
  exit 1
}

pkglibdir=$($pg_config --pkglibdir)
extension_dir=$($pg_config --sharedir)/extension
install -d "$pkglibdir" "$extension_dir"
install -m 0755 "$library" "$pkglibdir/openapi_fdw.so"
install -m 0644 "$control" "$extension_dir/openapi_fdw.control"
install -m 0644 "$sql" "$extension_dir/${sql##*/}"

printf 'Installed OpenAPI FDW %s for PostgreSQL %s. Run CREATE EXTENSION openapi_fdw in each target database.\n' \
  "$version" "$pg_major"
