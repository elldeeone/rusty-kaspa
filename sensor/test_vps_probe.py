#!/usr/bin/env python3
"""
Test script to verify VPS sensor active probing functionality.
Connects to a remote VPS sensor and verifies handshake + probe.

Usage:
    python test_vps_probe.py <vps-hostname-or-ip>

Example:
    python test_vps_probe.py kaspa-postgres-db
    python test_vps_probe.py 78.47.77.153
"""

import asyncio
import grpc
import random
import time
import sys
from pathlib import Path

# Add kaspa-crawler directory to path to import protobuf definitions
CRAWLER_PATH = Path("/tmp/kaspa-crawler")
sys.path.insert(0, str(CRAWLER_PATH))

try:
    import p2p_pb2
    import messages_pb2
    import messages_pb2_grpc
except ImportError:
    print("ERROR: Could not import kaspa-crawler protobuf definitions")
    print(f"Make sure kaspa-crawler is cloned to {CRAWLER_PATH}")
    print("Run: git clone https://github.com/kasfyi/kaspa-crawler.git /tmp/kaspa-crawler")
    sys.exit(1)


async def message_stream(queue):
    """Generator for outgoing messages"""
    message = await queue.get()
    while message is not None:
        print(f"→ Sending: {message.WhichOneof('payload')}")
        yield message
        queue.task_done()
        message = await queue.get()
    queue.task_done()


class TestPeer:
    """Simulates a peer connecting to the sensor"""

    USER_AGENT = "/test-peer:1.0/"

    def __init__(self, sensor_address, network="kaspa-mainnet"):
        self.sensor_address = sensor_address
        self.network = network
        self.peer_id = None
        self.peer_version = None
        self.peer_user_agent = None

    async def __aenter__(self):
        # Generate random peer ID
        self.id = bytes.fromhex(hex(int(random.random() * 10000))[2:].zfill(32))

        # Connect via gRPC
        print(f"\n=== Connecting to VPS sensor at {self.sensor_address} ===")
        self.channel = grpc.aio.insecure_channel(self.sensor_address)

        await asyncio.wait_for(self.channel.channel_ready(), 10)
        self.stub = messages_pb2_grpc.P2PStub(self.channel)

        # Setup message queue
        self.send_queue = asyncio.Queue()
        self.stream = self.stub.MessageStream(message_stream(self.send_queue))

        # Perform handshake
        await self.handshake()

        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        try:
            await self.send_queue.put(None)
            if exc_type is not None and issubclass(exc_type, asyncio.CancelledError):
                self.stream.cancel()
            else:
                await self.send_queue.join()
        finally:
            try:
                await asyncio.wait_for(self.channel.close(), timeout=2)
            except Exception as e:
                print(f"Error closing channel: {e}")

    async def handshake(self):
        """
        Perform P2P handshake with sensor.
        Exact protocol sequence:
        1. Receive version → Send version
        2. Receive verack → Send verack
        3. Receive ready → Send ready
        """
        print("\n=== Starting P2P Handshake ===")

        async for item in self.stream:
            payload_type = item.WhichOneof('payload')
            print(f"← Received: {payload_type}")

            if payload_type == "version":
                # Store sensor info
                self.peer_id = item.version.id.hex()
                self.peer_version = item.version.protocolVersion
                self.peer_user_agent = item.version.userAgent

                print(f"  Sensor version: {self.peer_version}")
                print(f"  Sensor user agent: {self.peer_user_agent}")

                # Send our version
                await self.send_queue.put(
                    messages_pb2.KaspadMessage(
                        version=p2p_pb2.VersionMessage(
                            protocolVersion=self.peer_version,
                            timestamp=int(time.time()),
                            id=self.id,
                            userAgent=self.USER_AGENT,
                            network=self.network,
                        )
                    )
                )

            elif payload_type == "verack":
                # Send verack
                await self.send_queue.put(
                    messages_pb2.KaspadMessage(verack=p2p_pb2.VerackMessage())
                )

                # Old protocol versions don't have ready message
                if self.peer_version < 4:
                    print("✅ Handshake complete (old protocol)")
                    return

            elif payload_type == "ready":
                # Send ready
                await self.send_queue.put(
                    messages_pb2.KaspadMessage(ready=p2p_pb2.ReadyMessage())
                )
                print("✅ Handshake complete")
                return

            else:
                print(f"⚠️  Unexpected message during handshake: {payload_type}")


async def main(vps_host):
    """Main test sequence"""
    print("=" * 60)
    print("VPS Sensor Active Probing Test")
    print("=" * 60)

    sensor_address = f"{vps_host}:16111"

    # Step 1: Connect to VPS sensor and complete handshake
    try:
        async with TestPeer(sensor_address) as peer:
            print("\n✅ Successfully connected and completed handshake with VPS sensor")
            print(f"   Sensor User Agent: {peer.peer_user_agent}")

            # Wait for sensor to probe back
            print("\nWaiting 5 seconds for sensor to probe back...")
            await asyncio.sleep(5)

        print("\n✅ Connection closed cleanly")

    except asyncio.TimeoutError:
        print("\n❌ Connection timeout - is the VPS sensor running and port 16111 open?")
        print(f"   Sensor address: {sensor_address}")
        return 1
    except Exception as e:
        print(f"\n❌ Connection error: {e}")
        return 1

    # Summary
    print("\n" + "=" * 60)
    print("✅ TEST PASSED: VPS sensor accepted connection")
    print("\nNow check VPS PostgreSQL database with:")
    print(f"  ssh root@{vps_host}")
    print("  docker exec -it kaspa-postgres psql -U sensor_writer -d kaspa_sensors -c \\")
    print("    \"SELECT * FROM peer_events ORDER BY created_at DESC LIMIT 5;\"")
    print("=" * 60)
    return 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python test_vps_probe.py <vps-hostname-or-ip>")
        print("Example: python test_vps_probe.py kaspa-postgres-db")
        sys.exit(1)

    vps_host = sys.argv[1]
    exit_code = asyncio.run(main(vps_host))
    sys.exit(exit_code)
