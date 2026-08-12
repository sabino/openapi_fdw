#!/bin/sh
set -eu

package_root=${1:?package export directory is required}
version=${2:?release version is required}
pg_major=${3:?PostgreSQL major is required}
output_dir=${4:-.}

case "$version" in
  v[0-9]*) ;;
  *)
    printf 'Release version must start with v and a digit: %s\n' "$version" >&2
    exit 1
    ;;
esac
case "$version" in
  *[!A-Za-z0-9._-]*)
    printf 'Release version contains unsupported characters: %s\n' "$version" >&2
    exit 1
    ;;
esac
case "$pg_major" in
  14|15|16|17|18) ;;
  *)
    printf 'Unsupported PostgreSQL major: %s\n' "$pg_major" >&2
    exit 1
    ;;
esac

library=$(find "$package_root" -type f -name 'openapi_fdw-*.so' -print -quit)
control=$(find "$package_root" -type f -name openapi_fdw.control -print -quit)
sql_files=$(find "$package_root" -type f -name 'openapi_fdw--*.sql' -print)

test -n "$library" && test -n "$control" && test -n "$sql_files" || {
  printf '%s\n' \
    'pgrx package export is incomplete; expected a versioned library, control file, and SQL file' >&2
  exit 1
}

mkdir -p "$output_dir"
staging=$(mktemp -d "${TMPDIR:-/tmp}/openapi-fdw-package.XXXXXX")
trap 'find "$staging" -xdev -depth -delete' EXIT HUP INT TERM

cp "$library" "$control" "$staging/"
for sql_file in $sql_files; do
  cp "$sql_file" "$staging/"
done
archive="openapi_fdw-${version}-pg${pg_major}-linux-amd64.tar.gz"
tar -C "$staging" -czf "$output_dir/$archive" .
(cd "$output_dir" && sha256sum "$archive" >"$archive.sha256")

printf '%s\n' "$output_dir/$archive" "$output_dir/$archive.sha256"
