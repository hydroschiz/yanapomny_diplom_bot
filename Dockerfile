# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy source code
COPY . .

# Build production service packages from bins/* and crates/*.
RUN cargo build --release --bins --workspace

# Final stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y ca-certificates libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy service binaries from builder
COPY --from=builder /app/target/release/bot-service .
COPY --from=builder /app/target/release/scheduler-service .
COPY --from=builder /app/target/release/webhook-service .

# Create directory for data if needed
RUN mkdir -p data

CMD ["./bot-service"]
