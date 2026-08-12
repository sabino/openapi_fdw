# Installation and deployment

OpenAPI FDW has two independent deployable artifacts:

- a PostgreSQL image or native extension package for the data plane; and
- an optional scratch control-plane image.

The PostgreSQL artifact must match both the PostgreSQL major and the target C
library. The published container tags use Alpine/musl. Published native
archives are built on Debian/glibc for x86-64.

## Docker Compose

For a local complete stack:

```bash
cp .env.example .env
openssl rand -hex 32
openssl rand -hex 32
# Put the two different results in POSTGRES_PASSWORD and
# OPENAPI_FDW_ADMIN_TOKEN, then:
docker compose up --build -d
docker compose ps
```

Open `http://localhost:8080`. Compose sets the session cookie to non-secure
only for local HTTP. It publishes PostgreSQL on port 5432 for local clients and
stores its cluster in the named `postgres-data` volume.

To stop without deleting data:

```bash
docker compose down
```

Do not add `--volumes` unless deleting the PostgreSQL cluster is intentional.

## Published images

The data-plane tags are:

```text
ghcr.io/sabino/openapi_fdw:pg18
ghcr.io/sabino/openapi_fdw:pg18-alpine
ghcr.io/sabino/openapi_fdw:v0.3.2-pg18
ghcr.io/sabino/openapi_fdw:v0.3.2-pg18-alpine
```

Replace `18` with 14, 15, 16, or 17 as needed. Floating `pgN` tags follow the
latest release; versioned tags are reproducible. The runtime layer contains the
official Alpine PostgreSQL image, CA certificates, and the extension package.
The Rust compiler and build cache stay in build stages.

The control-plane tags are:

```text
ghcr.io/sabino/openapi_fdw:control
ghcr.io/sabino/openapi_fdw:v0.3.2-control
```

It is a stripped static executable in `scratch`, runs as UID/GID 65532, embeds
all web assets, and has no shell or package manager.

## Run only PostgreSQL

```bash
docker volume create openapi-fdw-data
docker run --detach --name openapi-postgres \
  --env POSTGRES_USER=openapi_fdw \
  --env POSTGRES_DB=openapi_fdw \
  --env POSTGRES_PASSWORD='<long random password>' \
  --publish 5432:5432 \
  --volume openapi-fdw-data:/var/lib/postgresql \
  ghcr.io/sabino/openapi_fdw:pg18
```

Then connect and run `CREATE EXTENSION openapi_fdw;`. If an FDW option names a
secret environment variable, pass that variable to this PostgreSQL container,
not to the control-plane container.

## Production operation

Keep PostgreSQL on a private network unless direct client access is required,
persist `/var/lib/postgresql`, and use independent random values for the
database password and control-plane administrator token. Terminate HTTPS in
front of the control plane and retain its secure-cookie default.

Upgrades should keep PostgreSQL on the same major unless a normal PostgreSQL
major-version upgrade is performed. Updating a same-major image restarts the
service with the existing volume and updated extension files. Run
`ALTER EXTENSION openapi_fdw UPDATE;` when a future release introduces an
extension upgrade script.

Any PostgreSQL-compatible client can connect with the standard host, port,
database, user, and password fields. Prefer a dedicated read-only login and
grant it only `CONNECT`, schema `USAGE`, foreign-server `USAGE`, and `SELECT`
on the intended foreign tables. Each scan performs live outbound API work, so
client refresh frequency and upstream rate limits still matter.

## Checksummed native installation

The release page provides one archive and checksum per PostgreSQL major. On
glibc Linux x86-64 with the matching PostgreSQL runtime/development files:

```bash
curl -fsSL https://raw.githubusercontent.com/sabino/openapi_fdw/main/scripts/install.sh \
  | sudo sh -s -- --version v0.3.2 \
      --pg-config /usr/lib/postgresql/18/bin/pg_config
```

The script:

1. asks `pg_config` for the major and installation directories;
2. accepts only supported PostgreSQL 14 through 18 and x86-64;
3. downloads the matching GitHub release archive and SHA-256 file;
4. verifies the checksum; and
5. installs only the versioned `openapi_fdw-<version>.so` library, its control
   file, and versioned SQL file.

Omit `--version` to follow the latest GitHub release. Set `PG_CONFIG` or pass
`--pg-config` when several PostgreSQL installations exist. Build from source on
ARM64, Alpine, or another unsupported ABI.

## Build from source

Install Rust 1.88, `cargo-pgrx` 0.16.1, libclang, a C toolchain, CA
certificates, and server development files for the exact PostgreSQL major.
Then:

```bash
cargo install --locked cargo-pgrx --version 0.16.1
cargo pgrx init --pg18="$(command -v pg_config)"
cargo pgrx install --release --no-default-features --features pg18
```

Change both `--pg18` and `pg18` for another supported major. The Dockerfile is
also a reproducible multi-stage builder:

```bash
docker build --build-arg PG_MAJOR=18 -t openapi-fdw:pg18 .
docker build -f Dockerfile.control -t openapi-fdw:control .
```

`POSTGRES_VARIANT=bookworm` selects the glibc package builder; Alpine is the
runtime-image default.

## Production checklist

- Keep the PostgreSQL port private unless external SQL access is required and
  separately protected.
- Put the control plane behind HTTPS and use an independent random token.
- Prefer environment-backed API credentials and grant foreign schemas only to
  intended roles.
- Apply CPU, memory, outbound-network, and connection limits appropriate to the
  upstream APIs.
- Back up PostgreSQL metadata and any local tables; the remote API rows
  themselves are not copied by this project.
- Pin versioned images where reproducibility matters and review release notes
  before changing PostgreSQL or extension versions.
