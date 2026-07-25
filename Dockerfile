FROM rust:1.89-slim-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev git \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Warm the dependency cache against the manifests alone, so editing src/ does not rebuild the
# whole turbomcp tree.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

FROM debian:bookworm-slim

# nsjail is deliberately absent: it is not packaged for Debian, and running it inside Docker needs
# CAP_SYS_ADMIN — which would weaken the container boundary more than the per-session jail
# strengthens it. The container *is* the sandbox here: unprivileged user, no capabilities, and
# only /workspace writable. See ARCHITECTURE.md, "Isolation model".
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl python3 python3-venv \
    && rm -rf /var/lib/apt/lists/*

# Debian 12 marks the system interpreter externally-managed (PEP 668), so `pip install` into it
# fails outright. A venv is what makes install_package work at all, and it keeps agent-installed
# packages out of the OS interpreter.
ENV VIRTUAL_ENV=/opt/venv
RUN python3 -m venv "$VIRTUAL_ENV" && "$VIRTUAL_ENV/bin/pip" install --no-cache-dir --upgrade pip

COPY --from=builder /app/target/release/liberado-python-interpreter-mcp /usr/local/bin/
COPY sandbox /usr/local/lib/liberado-python-interpreter-mcp/sandbox

ARG APP_UID=10001
RUN useradd --uid "$APP_UID" --create-home --shell /usr/sbin/nologin interpreter \
    && mkdir -p /workspace \
    && chown -R "$APP_UID:$APP_UID" /workspace "$VIRTUAL_ENV"

# The rootfs is read-only in deployment, so pip's cache directory is unwritable; without this it
# warns on every install. Nothing is lost — the cache could never have persisted anyway.
ENV PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    LIBERADO_WRAPPER_PATH=/usr/local/lib/liberado-python-interpreter-mcp/sandbox/wrapper.py \
    LIBERADO_WORKSPACE_ROOTS=/workspace \
    LIBERADO_SANDBOX_ENABLED=0 \
    SYSTEM_PYTHON=/opt/venv/bin/python \
    SANDBOX_PYTHON=/opt/venv/bin/python \
    BIND_ADDR=0.0.0.0:8000 \
    RUST_LOG=info

USER interpreter
WORKDIR /workspace
EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD curl -s -o /dev/null http://localhost:8000/ || exit 1

CMD ["liberado-python-interpreter-mcp"]
