# syntax=docker/dockerfile:1.7
ARG PG_MAJOR=18
ARG POSTGRES_VARIANT=alpine

FROM postgres:${PG_MAJOR}-${POSTGRES_VARIANT} AS toolchain
ARG PG_MAJOR

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/root/.cargo/bin:${PATH}

RUN if command -v apk >/dev/null 2>&1; then \
        apk add --no-cache build-base ca-certificates clang21 clang21-libclang \
            cmake curl llvm21-dev openssl-dev openssl-libs-static perl pkgconf; \
    else \
        apt-get update \
        && apt-get install -y --no-install-recommends \
            build-essential ca-certificates clang curl libclang-dev pkg-config \
            postgresql-server-dev-${PG_MAJOR} \
        && rm -rf /var/lib/apt/lists/*; \
    fi

RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
        https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --profile minimal --default-toolchain 1.88.0 \
    && rustup component add rustfmt \
    && cargo install --locked cargo-pgrx --version 0.16.1

# A tiny export target used by constrained development hosts. It copies only
# the pgrx driver, not the compiler image or Cargo target directory.
FROM scratch AS cargo-pgrx-export
COPY --from=toolchain /root/.cargo/bin/cargo-pgrx /cargo-pgrx

FROM toolchain AS builder
ARG PG_MAJOR

# Rust's musl target is fully static by default. pgrx runs bindgen while it is
# compiling, so its build script must be able to dlopen Alpine's libclang.
# Disabling only the C-runtime static feature keeps the exported cargo-pgrx
# driver portable while allowing the extension build to load libclang.
ENV PATH=/root/.cargo/bin:${PATH} \
    RUSTFLAGS="-C target-feature=-crt-static"

WORKDIR /build
COPY Cargo.toml Cargo.lock openapi_fdw.control ./
COPY sql ./sql
COPY src ./src
COPY control-plane ./control-plane

RUN --mount=type=cache,id=openapi-fdw-pg${PG_MAJOR}-target,target=/build/target,sharing=locked \
    if command -v apk >/dev/null 2>&1; then \
        export CLANG_PATH=/usr/bin/clang-21 \
            LIBCLANG_PATH=/usr/lib/llvm21/lib \
            LLVM_CONFIG_PATH=/usr/bin/llvm-config-21; \
    fi; \
    cargo pgrx init --pg${PG_MAJOR}="$(command -v pg_config)" \
    && cargo pgrx package --no-default-features --features pg${PG_MAJOR} \
    && mkdir -p /package \
    && cp -a target/release/openapi_fdw-pg${PG_MAJOR}/usr/. /package/

FROM scratch AS extension-package
COPY --from=builder /package/ /

FROM postgres:${PG_MAJOR}-${POSTGRES_VARIANT}
ARG PG_MAJOR

RUN if command -v apk >/dev/null 2>&1; then \
        apk add --no-cache ca-certificates; \
    else \
        apt-get update \
        && apt-get install -y --no-install-recommends ca-certificates \
        && rm -rf /var/lib/apt/lists/*; \
    fi

COPY --from=builder /package/ /usr/

HEALTHCHECK --interval=5s --timeout=3s --start-period=15s --retries=12 \
    CMD pg_isready -U "${POSTGRES_USER:-postgres}" || exit 1

LABEL org.opencontainers.image.source="https://github.com/sabino/openapi_fdw" \
      org.opencontainers.image.description="PostgreSQL ${PG_MAJOR} with the native OpenAPI FDW" \
      org.opencontainers.image.version="0.3.1"
