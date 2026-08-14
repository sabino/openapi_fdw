#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/openapi-fdw-install-test.XXXXXX")
trap 'find "$temporary" -xdev -depth -delete' EXIT HUP INT TERM

mkdir -p "$temporary/export/usr/lib/postgresql/18/lib"
mkdir -p "$temporary/export/usr/share/postgresql/18/extension"
mkdir -p "$temporary/release" "$temporary/install"

printf 'library\n' >"$temporary/export/usr/lib/postgresql/18/lib/openapi_fdw-0.4.0.so"
printf 'control\n' >"$temporary/export/usr/share/postgresql/18/extension/openapi_fdw.control"
printf 'sql\n' >"$temporary/export/usr/share/postgresql/18/extension/openapi_fdw--0.4.0.sql"
printf 'old-upgrade\n' >"$temporary/export/usr/share/postgresql/18/extension/openapi_fdw--0.3.1--0.3.2.sql"
printf 'upgrade\n' >"$temporary/export/usr/share/postgresql/18/extension/openapi_fdw--0.3.2--0.4.0.sql"

"$repository_root/scripts/package-native.sh" \
  "$temporary/export" v0.4.0 18 "$temporary/release" >/dev/null

archive="$temporary/release/openapi_fdw-v0.4.0-pg18-linux-amd64.tar.gz"
export TEST_NATIVE_ARCHIVE="$archive"
export TEST_NATIVE_CHECKSUM="$archive.sha256"
export TEST_NATIVE_INSTALL_ROOT="$temporary/install"

PATH="$repository_root/tests/fixtures/native-install:$PATH" \
  "$repository_root/scripts/install.sh" \
    --version v0.4.0 \
    --pg-config "$repository_root/tests/fixtures/native-install/pg_config" \
    >/dev/null

test -f "$temporary/install/lib/openapi_fdw-0.4.0.so"
test ! -e "$temporary/install/lib/openapi_fdw.so"
test -f "$temporary/install/share/extension/openapi_fdw.control"
test -f "$temporary/install/share/extension/openapi_fdw--0.4.0.sql"
test -f "$temporary/install/share/extension/openapi_fdw--0.3.1--0.3.2.sql"
test -f "$temporary/install/share/extension/openapi_fdw--0.3.2--0.4.0.sql"

printf '%s\n' 'native installer contract passed'
