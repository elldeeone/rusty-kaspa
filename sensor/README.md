# Kaspa Network Sensor

Lightweight P2P network sensor for mapping Kaspa's network topology by classifying nodes as Public (listening) or Private (NAT/firewall).

## What It Does

1. **Accepts P2P Connections** - Listens on port 16111 for inbound peers
2. **Completes Handshakes** - Implements Kaspa's P2P protocol
3. **Classifies Peers** - Actively probes peers to determine if they're publicly reachable
4. **Logs Everything** - Stores all connection events in SQLite with full metadata
5. **Exports Data** - Sends batched events to HTTP/Firebase endpoints

## Quick Start

```bash
# Build
cargo build --release -p kaspa-sensor

# Generate config
./target/release/kaspa-sensor --generate-config

# Run
./target/release/kaspa-sensor -c sensor.toml --sensor-id my-sensor
```

## Configuration

Edit `sensor.toml`:

```toml
[sensor]
sensor_id = "sensor-001"

[network]
listen_address = "0.0.0.0:16111"
network_type = "mainnet"

[probing]
enabled = true
timeout_ms = 5000

[export]
enabled = false  # Set true for production
backend = "http"
endpoint = "https://api.example.com/events"

[database]
path = "sensor-data.db"
addressdb_path = "sensor-addresses.db"

[metrics]
enabled = true
address = "0.0.0.0:9090"
```

## Data Model

Each peer connection logs:
- Timestamp, sensor ID, peer IP/port
- Protocol version, user agent
- Direction (inbound/outbound)
- Classification (public/private/unknown)
- Probe duration and any errors

## Metrics

Prometheus metrics at `http://localhost:9090/metrics`:
- `sensor_connections_total` - Total connections
- `sensor_probes_total` - Probes by classification
- `sensor_peers_classified_public_total` - Public peer count
- `sensor_peers_classified_private_total` - Private peer count

## Testing

Run the included test script to verify Phase 2 active probing:

```bash
cd sensor
python3 test_probe.py
```

This simulates an inbound connection and verifies the sensor:
1. Accepts the connection
2. Completes the P2P handshake
3. Automatically probes back
4. Stores the event in the database

## Phase 3: Network Gossip Analysis

See [PHASE3_GUIDE.md](PHASE3_GUIDE.md) for instructions on using the Python kaspa-crawler to verify sensor IP propagation across the network.

## Deployment

For production, deploy multiple sensors across different regions/providers:

```bash
# AWS us-east-1
./kaspa-sensor --sensor-id aws-us-east-1

# DigitalOcean sgp1
./kaspa-sensor --sensor-id do-sgp1

# Hetzner eu-central
./kaspa-sensor --sensor-id hetzner-eu
```

Each sensor operates independently with local storage and exports to centralized collection.

## Docker

```bash
# Build
docker build -t kaspa-sensor -f sensor/Dockerfile .

# Run
docker run -d \
  -p 16111:16111 \
  -p 9090:9090 \
  -v $(pwd)/sensor.toml:/app/sensor.toml:ro \
  --name kaspa-sensor \
  kaspa-sensor
```

## Project Structure

```
sensor/
├── src/
│   ├── main.rs      # Entry point, P2P connection handling
│   ├── config.rs    # Configuration
│   ├── models.rs    # Data models
│   ├── storage.rs   # SQLite persistence
│   ├── export.rs    # HTTP/Firebase export
│   ├── prober.rs    # Active peer probing
│   └── metrics.rs   # Prometheus metrics
├── test_probe.py    # Phase 2 verification script
└── PHASE3_GUIDE.md  # Network gossip analysis guide
```

## License

MIT OR Apache-2.0
