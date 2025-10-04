# Build stage
FROM rust:1.82 AS builder

WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY clippy.toml .rustfmt.toml ./

# Copy all crates (sensor depends on many workspace crates)
COPY core ./core
COPY crypto ./crypto
COPY consensus ./consensus
COPY database ./database
COPY mining ./mining
COPY notify ./notify
COPY protocol ./protocol
COPY utils ./utils
COPY components ./components
COPY metrics ./metrics
COPY math ./math
COPY sensor ./sensor

# Build in release mode
RUN cargo build --release --bin kaspa-sensor

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN useradd -m -u 1000 sensor && \
    mkdir -p /data && \
    chown -R sensor:sensor /data

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/target/release/kaspa-sensor /app/kaspa-sensor

# Set ownership
RUN chown -R sensor:sensor /app

# Switch to non-root user
USER sensor

# Data volume for database and logs
VOLUME ["/data"]

# Expose ports
# 16111 - P2P port
# 9090 - Metrics port
EXPOSE 16111 9090

# Environment variables
ENV RUST_LOG=info
ENV SENSOR_DATA_DIR=/data

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD [ "sh", "-c", "netstat -an | grep 16111 > /dev/null || exit 1" ]

# Run the sensor
ENTRYPOINT ["/app/kaspa-sensor"]
CMD ["--config", "/data/sensor.toml"]
