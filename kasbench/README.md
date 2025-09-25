# Kasbench - Kaspa Performance Benchmarking Platform

A high-performance transaction generation and benchmarking platform for the Kaspa network.

## Features
- Achieve 1000+ TPS with optimized transaction generation
- Simple web dashboard for real-time monitoring
- Docker-based deployment for consistent environment
- Automatic configuration and optimization
- CSV export for results verification

## Quick Start

```bash
# 1. Clone the repository
git clone https://github.com/kaspanet/rusty-kaspa
cd rusty-kaspa/kasbench

# 2. Start the benchmark environment
docker-compose up

# 3. Open dashboard
# Navigate to http://localhost:8080
```

## Architecture
- **Kaspa Node**: Stock standard kaspanet Docker image
- **Kasbench Engine**: Enhanced Rothschild-based transaction generator
- **Dashboard**: Simple web interface for monitoring and control

## Performance
Designed to achieve maximum TPS on standard hardware:
- Tested up to 2000+ TPS on local networks
- Automatic optimization based on system resources
- Intelligent UTXO management for sustained performance