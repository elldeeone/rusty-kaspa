# Multi-stage build for kasbench server
FROM rust:1.81 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p kasbench

FROM debian:bookworm-slim
ENV RUST_LOG=info
RUN useradd -m -u 10001 appuser && mkdir -p /data && chown -R appuser:appuser /data
USER appuser
WORKDIR /home/appuser
COPY --from=builder /app/target/release/kasbench-engine /usr/local/bin/kasbench
VOLUME ["/data"]
EXPOSE 8080
# RPC_SERVER and NETWORK should be set by the user
ENV RPC_SERVER="grpc://host.docker.internal:16110" \
    NETWORK="mainnet" \
    KASBENCH_ENABLE_UI="0"

CMD ["/usr/local/bin/kasbench","serve","--bind","0.0.0.0","--port","8080"]

