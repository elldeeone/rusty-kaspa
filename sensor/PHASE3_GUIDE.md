# Phase 3: Network Gossip Analysis Using Python Crawler

## Overview

Phase 3 answers the critical question: **"Are our sensors gossiped across the network?"**

Instead of reimplementing the crawler in Rust, we use the battle-tested Python [kaspa-crawler](https://github.com/kasfyi/kaspa-crawler.git) which already implements the exact P2P protocol we need.

## Why Python Instead of Rust?

The Python kaspa-crawler:
- ✅ Already works perfectly with Kaspa P2P protocol
- ✅ Uses gRPC directly for low-level message exchange
- ✅ Implements exact handshake sequence (version → verack → ready → getaddr)
- ✅ Battle-tested in production
- ✅ Simple to modify for our specific needs

Reimplementing this in Rust would require:
- ❌ Adding gRPC dependencies (tonic)
- ❌ Using protobuf definitions directly
- ❌ Implementing raw message streaming
- ❌ Duplicating working code
- ❌ 3-5 days of development

## Prerequisites

1. **Deploy Sensors** (Phases 1 & 2)
   ```bash
   # Generate config
   ./target/release/kaspa-sensor --generate-config

   # Deploy sensor (note the external IP!)
   ./target/release/kaspa-sensor -c sensor.toml --sensor-id sensor-aws-us-east-1

   # Get your sensor's public IP
   curl ifconfig.me
   # Example output: 44.201.123.45
   ```

2. **Let it Run**: Wait 24-48 hours for sensors to be discovered by the network

3. **Install Python Crawler**
   ```bash
   git clone https://github.com/kasfyi/kaspa-crawler.git
   cd kaspa-crawler
   pip install -r requirements.txt  # or use virtual env
   ```

## Running the Crawler

### Basic Crawl

Crawl the network starting from DNS seeders:

```bash
python kaspa_crawler.py \
    --addr seeder1.kaspad.net:16111 \
    --addr seeder2.kaspad.net:16111 \
    --network kaspa-mainnet \
    --output crawler-results.db
```

This will:
1. Connect to seed nodes
2. Send `RequestAddresses` (getaddr) messages
3. Collect `Addresses` responses
4. Recursively crawl discovered peers
5. Store results in SQLite database

### Check for Sensor IPs

After the crawl completes, query the database:

```bash
# Check if your sensor IP appears in collected addresses
sqlite3 crawler-results.db "
SELECT DISTINCT ip, kaspad, neighbor
FROM (
    SELECT ip, kaspad, json_each.value as neighbor
    FROM nodes, json_each(neighbors)
)
WHERE neighbor LIKE '%44.201.123.45%'
"
```

**If results found**: ✅ Your sensor IS being gossiped!
**If no results**: ❌ Your sensor is NOT being gossiped

## Modifying the Crawler for Sensor Tracking

Create a modified version that specifically tracks sensor IPs:

```python
# sensor_tracker.py
import sys
from kaspa_crawler import main
import asyncio
import sqlite3

SENSOR_IPS = [
    "44.201.123.45",  # sensor-aws-us-east-1
    "35.180.92.11",   # sensor-aws-eu-west-3
    "3.68.128.228",   # sensor-aws-eu-central-1
]

async def track_sensors():
    # Run the crawler
    seed_nodes = [
        ("seeder1.kaspad.net", "16111"),
        ("seeder2.kaspad.net", "16111"),
    ]

    await main(seed_nodes, "kaspa-mainnet", "sensor-tracking.db")

    # Analyze results
    conn = sqlite3.connect("sensor-tracking.db")
    cursor = conn.cursor()

    print("\n=== Sensor Propagation Analysis ===\n")

    for sensor_ip in SENSOR_IPS:
        # Query which peers advertised this sensor
        cursor.execute("""
            SELECT ip, kaspad
            FROM nodes, json_each(neighbors)
            WHERE json_each.value LIKE ?
        """, (f'%{sensor_ip}%',))

        advertisers = cursor.fetchall()

        if advertisers:
            print(f"✅ Sensor {sensor_ip}: FOUND")
            print(f"   Advertised by {len(advertisers)} peer(s):")
            for peer_ip, user_agent in advertisers[:5]:  # Show first 5
                print(f"   - {peer_ip} ({user_agent})")
        else:
            print(f"❌ Sensor {sensor_ip}: NOT FOUND")
        print()

    conn.close()

if __name__ == "__main__":
    asyncio.run(track_sensors())
```

Run it:

```bash
python sensor_tracker.py
```

## Generating the Phase 3 Report

Based on crawler results, create a report:

```markdown
# Phase 3 Analysis Report

**Date**: 2025-10-05
**Duration**: 48 hours
**Sensors Deployed**: 3
**Network**: Mainnet

## Results

### Sensor 1: 44.201.123.45 (aws-us-east-1)
- **Status**: ✅ FOUND
- **Advertised by**: 47 unique peers (23% of 204 crawled)
- **First seen**: 2025-10-03 14:22 UTC (6 hours after deployment)
- **Propagation**: HIGH

### Sensor 2: 35.180.92.11 (aws-eu-west-3)
- **Status**: ✅ FOUND
- **Advertised by**: 52 unique peers (25% of 204 crawled)
- **First seen**: 2025-10-03 15:10 UTC (7 hours after deployment)
- **Propagation**: HIGH

### Sensor 3: 3.68.128.228 (aws-eu-central-1)
- **Status**: ✅ FOUND
- **Advertised by**: 39 unique peers (19% of 204 crawled)
- **First seen**: 2025-10-03 16:45 UTC (9 hours after deployment)
- **Propagation**: MEDIUM-HIGH

## Answer to Research Question

**Question**: Will a node that only completes the P2P handshake (and does not relay blocks, transactions, or respond to data requests) be considered an 'active' peer by other nodes and have its address propagated via addr messages?

**Answer**: ✅ **YES**

**Evidence**:
- All 3 deployed sensors appeared in address gossip
- Average propagation rate: 22.3% of crawled peers
- Propagation time: 6-9 hours after deployment
- Protocol skeleton nodes ARE added to peer lists and gossiped

## Deployment Implications

✅ **Moderate-scale deployment is sufficient** (10-20 sensors)
✅ Sensors will organically reach most of the network via gossip
✅ DNS seeders provide initial discovery, gossip provides propagation
✅ Geographic distribution recommended for best coverage

## Recommendations

1. Deploy 10-15 sensors across different regions/ASNs
2. Monitor propagation with periodic crawler runs
3. Scale up if coverage gaps identified
4. Use Prometheus federation for centralized metrics
```

## Alternative: Manual Analysis

If you prefer not to modify the Python code:

```bash
# 1. Run crawler
python kaspa_crawler.py --addr seeder1.kaspad.net:16111 --output results.db

# 2. Extract all advertised IPs to a file
sqlite3 results.db "
SELECT DISTINCT json_each.value
FROM nodes, json_each(neighbors)
" > all_advertised_ips.txt

# 3. Check if your sensor IPs are in there
grep "44.201.123.45" all_advertised_ips.txt && echo "FOUND!" || echo "NOT FOUND"

# 4. Count how many peers advertised it
sqlite3 results.db "
SELECT COUNT(DISTINCT ip)
FROM nodes, json_each(neighbors)
WHERE json_each.value LIKE '%44.201.123.45%'
"
```

## Deployment Timeline

**Day 0**: Deploy sensors
**Day 1-2**: Sensors accept connections, get added to peer address books
**Day 2**: Run crawler to check gossip
**Day 3**: Analyze results and generate report

## Key Metrics

- **Propagation Rate**: % of crawled peers that advertise sensor IP
- **Time to Propagation**: Hours from deployment to first gossip
- **Coverage**: Number of unique peers advertising sensor
- **Geographic Spread**: Distribution of advertisers across ASNs

## Expected Outcomes

**If sensors ARE gossiped (likely)**:
- 15-30% of peers will advertise sensor IPs
- Propagation time: 6-12 hours
- Deployment strategy: 10-20 sensors for broad coverage

**If sensors are NOT gossiped (unlikely)**:
- 0% of peers advertise sensor IPs
- No propagation observed
- Deployment strategy: 100+ sensors needed, rely on DNS seeders

---

**Note**: The Python kaspa-crawler is the official, tested tool for this analysis. Using it for Phase 3 is the pragmatic choice.
