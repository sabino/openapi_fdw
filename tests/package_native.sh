#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/openapi-fdw-package-test.XXXXXX")
trap 'find "$temporary" -xdev -depth -delete' EXIT HUP INT TERM

mkdir -p "$temporary/export/usr/lib/postgresql/18/lib"
mkdir -p "$temporary/export/usr/share/postgresql/18/extension"
mkdir -p "$temporary/output" "$temporary/unpacked"

printf 'library\n' >"$temporary/export/usr/lib/postgresql/18/lib/openapi_fdw-0.3.1.so"
printf 'control\n' >"$temporary/export/usr/share/postgresql/18/extension/openapi_fdw.control"
printf 'sql\n' >"$temporary/export/usr/share/postgresql/18/extension/openapi_fdw--0.3.1.sql"

"$repository_root/scripts/package-native.sh" \
  "$temporary/export" v0.3.1 18 "$temporary/output" >/dev/null

archive="$temporary/output/openapi_fdw-v0.3.1-pg18-linux-amd64.tar.gz"
(cd "$temporary/output" && sha256sum --check "${archive##*/}.sha256" >/dev/null)
tar -C "$temporary/unpacked" -xzf "$archive"

test -f "$temporary/unpacked/openapi_fdw-0.3.1.so"
test -f "$temporary/unpacked/openapi_fdw.control"
test -f "$temporary/unpacked/openapi_fdw--0.3.1.sql"
test ! -e "$temporary/unpacked/openapi_fdw.so"

printf '%s\n' 'native package contract passed'
