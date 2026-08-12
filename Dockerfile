# syntax=docker/dockerfile:1.7
ARG PG_MAJOR=18

FROM postgres:${PG_MAJOR}-bookworm AS toolchain
ARG PG_MAJOR

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH=/root/.cargo/bin:/usr/lib/postgresql/${PG_MAJOR}/bin:${PATH}

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        curl \
        libclang-dev \
        pkg-config \
        postgresql-server-dev-${PG_MAJOR} \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
        https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --profile minimal --default-toolchain 1.88.0 \
    && cargo install --locked cargo-pgrx --version 0.16.1

# A tiny export target used by constrained development hosts. It copies only
# the pgrx driver, not the compiler image or Cargo target directory.
FROM scratch AS cargo-pgrx-export
COPY --from=toolchain /root/.cargo/bin/cargo-pgrx /cargo-pgrx

FROM toolchain AS builder
ARG PG_MAJOR

ENV PATH=/root/.cargo/bin:/usr/lib/postgresql/${PG_MAJOR}/bin:${PATH}

WORKDIR /build
COPY Cargo.toml Cargo.lock openapi_fdw.control ./
COPY sql ./sql
COPY src ./src

RUN --mount=type=cache,id=openapi-fdw-pg${PG_MAJOR}-target,target=/build/target,sharing=locked \
    cargo pgrx init --pg${PG_MAJOR}=/usr/lib/postgresql/${PG_MAJOR}/bin/pg_config \
    && cargo pgrx package --no-default-features --features pg${PG_MAJOR} \
    && mkdir -p /package \
    && cp -a target/release/openapi_fdw-pg${PG_MAJOR}/usr/. /package/

FROM postgres:${PG_MAJOR}-bookworm
ARG PG_MAJOR

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /package/ /usr/

HEALTHCHECK --interval=5s --timeout=3s --start-period=15s --retries=12 \
    CMD pg_isready -U "${POSTGRES_USER:-postgres}" || exit 1

LABEL org.opencontainers.image.source="https://github.com/sabino/openapi_fdw" \
      org.opencontainers.image.description="PostgreSQL ${PG_MAJOR} with the native OpenAPI FDW"
