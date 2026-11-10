# syntax=docker/dockerfile:1.7

# ---------- build stage -------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /app

# Workspace manifests and sources. Registry and target directories are BuildKit
# cache mounts, so unchanged dependencies are not rebuilt between builds.
COPY Cargo.toml Cargo.lock ./
COPY common ./common
COPY datasources ./datasources
COPY sql-portal ./sql-portal
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin script-portal \
    && cp target/release/script-portal /usr/local/bin/script-portal

# ---------- runtime stage -----------------------------------------------------
# distroless/cc: glibc + CA certificates, no shell, runs as non-root.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

ARG VERSION=dev
LABEL org.opencontainers.image.title="script-portal" \
      org.opencontainers.image.description="Config-driven SQL script portal with LDAP/OIDC, forms and charts" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.licenses="MIT"

WORKDIR /app
COPY --from=builder /usr/local/bin/script-portal /usr/local/bin/script-portal
COPY config/ /app/config/

# The bundled demo config DISABLES authentication and uses an in-memory SQLite
# database. Mount your own configuration and set SCRIPT_PORTAL_CONFIG for real use.
ENV SCRIPT_PORTAL_CONFIG=/app/config/config.demo.yaml \
    SCRIPT_PORTAL_BIND=0.0.0.0:8080 \
    RUST_LOG=info

EXPOSE 8080
USER nonroot:nonroot

# Distroless has no curl: the binary ships its own probe (GET /readyz).
HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/script-portal", "healthcheck", "--addr", "127.0.0.1:8080", "--path", "/readyz"]

ENTRYPOINT ["/usr/local/bin/script-portal"]
CMD ["serve"]
