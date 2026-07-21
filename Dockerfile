FROM rust:1.89-slim-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs && mkdir sandbox && touch sandbox/wrapper.py && cargo build --release && rm -rf src sandbox
COPY src ./src
COPY sandbox ./sandbox
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl python3 python3-pip nsjail \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/liberado-python-interpreter-mcp /usr/local/bin/
COPY --from=builder /app/sandbox /usr/local/lib/liberado-python-interpreter-mcp/sandbox
ENV LIBERADO_WRAPPER_PATH=/usr/local/lib/liberado-python-interpreter-mcp/sandbox/wrapper.py
EXPOSE 8000
CMD ["liberado-python-interpreter-mcp"]
