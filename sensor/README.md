# Kaspa Network Sensor

A lightweight, distributed network sensor designed to map the topology of the Kaspa peer-to-peer (P2P) network. This tool empirically identifies and classifies all connecting nodes as either **Public (Listening)** or **Private (Non-Listening)** to gather high-fidelity data on the true size and composition of the network.

## Features

### Phase 1 & 2: Complete ✅
- **Passive P2P Listener**: Accepts inbound connections on port 16111
- **Protocol Handshake**: Implements Kaspa's P2P handshake protocol
- **Active Peer Probing**: Classifies peers as Public/Private via TCP connection attempts
- **Structured Data Logging**: Records all peer interactions with full metadata
- **Local SQLite Storage**: Offline-resilient data persistence
- **Data Export**: Batched export to HTTP endpoints or Firebase/Firestore
- **Prometheus Metrics**: Comprehensive monitoring and observability
- **Rate Limiting**: Configurable concurrency controls for probing
- **Graceful Shutdown**: Clean connection termination and data flushing

### Phase 3: Planned 🚧
- Network crawler for gossip analysis
- Peer address propagation tracking

## Architecture

The sensor is a **"protocol skeleton"** node - it participates in P2P handshakes but doesn't sync the DAG, validate transactions, or participate in consensus. This minimal footprint allows for:

- ✅ Extremely low resource usage (< 100MB RAM)
- ✅ Massively scalable deployment (100s of instances)
- ✅ Cost-effective distributed monitoring

## Quick Start

### Prerequisites

- Rust 1.82.0+
- Docker (optional, for containerized deployment)

### Local Build & Run

```bash
# Generate default configuration
cargo run --release --bin kaspa-sensor -- --generate-config

# Edit sensor.toml with your settings
nano sensor.toml

# Run the sensor
cargo run --release --bin kaspa-sensor -- --config sensor.toml
```

### Docker Deployment

```bash
# Build image
docker build -t kaspa-sensor -f sensor/Dockerfile .

# Run single instance
docker run -d \
  -p 16111:16111 \
  -p 9090:9090 \
  -v $(pwd)/data:/data \
  -v $(pwd)/sensor.toml:/data/sensor.toml:ro \
  --name kaspa-sensor-1 \
  kaspa-sensor

# Or use docker-compose for multi-sensor setup
cd sensor
docker-compose up -d
```

## Configuration

See `sensor.toml.example` for a complete configuration template.

### Key Settings

| Section | Parameter | Description |
|---------|-----------|-------------|
| `[sensor]` | `sensor_id` | Unique identifier for this instance |
| `[network]` | `listen_address` | P2P listen address (default: 0.0.0.0:16111) |
| `[network]` | `network_type` | mainnet, testnet, or devnet |
| `[probing]` | `enabled` | Enable active peer classification |
| `[probing]` | `timeout_ms` | Probe timeout in milliseconds |
| `[export]` | `backend` | Export backend: http, firestore, webhook |
| `[export]` | `endpoint` | Data export URL |
| `[database]` | `path` | SQLite database file path |
| `[metrics]` | `address` | Prometheus metrics endpoint |

## Data Model

### Peer Connection Event

Each peer interaction is logged with:

```json
{
  "event_id": "uuid",
  "timestamp": "2025-01-15T12:00:00Z",
  "sensor_id": "sensor-001",
  "peer_ip": "203.0.113.1",
  "peer_port": 16111,
  "protocol_version": 7,
  "user_agent": "kaspad:0.14.0",
  "network": "kaspa-mainnet",
  "direction": "inbound",
  "classification": "public",
  "probe_duration_ms": 250,
  "probe_error": null
}
```

## Metrics

Access Prometheus metrics at `http://localhost:9090/metrics`

Key metrics:
- `sensor_connections_total` - Total connections by direction/result
- `sensor_probes_total` - Total probes by classification
- `sensor_probe_duration_seconds` - Probe duration histogram
- `sensor_peers_classified_public_total` - Public peer count
- `sensor_peers_classified_private_total` - Private peer count
- `sensor_exports_total` - Export attempts
- `sensor_storage_events_total` - Events in database

## Deployment Strategies

### Single Sensor (Development)
```bash
cargo run --release --bin kaspa-sensor
```

### Multi-Sensor Fleet (Production)

Deploy across multiple geographic regions and networks for comprehensive coverage:

```yaml
# Example: 3 sensors across different providers
- sensor-1: AWS us-east-1
- sensor-2: DigitalOcean sgp1
- sensor-3: Hetzner eu-central
```

Each sensor operates independently with local storage and exports to a centralized data warehouse.

## Export Backends

### HTTP/Webhook
```toml
[export]
backend = "http"
endpoint = "https://api.example.com/sensor-data"
api_key = "your-api-key"
```

### Firebase/Firestore
```toml
[export]
backend = "firestore"
endpoint = "https://firestore.googleapis.com/v1/projects/PROJECT/databases/(default)/documents/peers"
api_key = "your-firebase-api-key"
```

## Monitoring Setup

The included docker-compose setup provides:
- **Prometheus**: Metrics collection
- **Grafana**: Visualization dashboards (http://localhost:3000, admin/admin)

## Troubleshooting

### Port Already in Use
```bash
# Check what's using port 16111
lsof -i :16111

# Or use a different port
kaspa-sensor --listen 0.0.0.0:16112
```

### Database Locked
Ensure only one sensor instance uses a database file:
```toml
[database]
path = "/data/sensor-001.db"
```

### High Memory Usage
Reduce concurrent connections:
```toml
[network]
max_inbound_connections = 100

[probing]
max_concurrent_probes = 50
```

## Development

```bash
# Run tests
cargo test --package kaspa-sensor

# Format code
cargo fmt --all

# Lint
cargo clippy --workspace

# Build documentation
cargo doc --package kaspa-sensor --open
```

## Project Structure

```
sensor/
├── src/
│   ├── main.rs          # Application entry point
│   ├── lib.rs           # Library exports
│   ├── config.rs        # Configuration management
│   ├── models.rs        # Data models
│   ├── storage.rs       # SQLite persistence
│   ├── export.rs        # Data export backends
│   ├── prober.rs        # Active peer probing
│   └── metrics.rs       # Prometheus metrics
├── Cargo.toml           # Dependencies
├── Dockerfile           # Container image
├── docker-compose.yml   # Multi-sensor orchestration
├── sensor.toml.example  # Configuration template
└── README.md            # This file
```

## Contributing

This sensor is part of the larger Kaspa network research initiative. For bugs or feature requests, please open an issue on the main rusty-kaspa repository.

## License

MIT OR Apache-2.0

## Acknowledgments

Built using the [rusty-kaspa](https://github.com/kaspanet/rusty-kaspa) P2P library and protocol implementations.
