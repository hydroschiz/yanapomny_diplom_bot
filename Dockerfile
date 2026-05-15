# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy source code
COPY . .

# Build all runtime entrypoints
RUN cargo build --release --bins

# Final stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y ca-certificates libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/target/release/yanapomnyu_bot .
COPY --from=builder /app/target/release/scheduler .
COPY --from=builder /app/target/release/webhook .

# Create directory for data if needed
RUN mkdir -p data

CMD ["./yanapomnyu_bot"]
