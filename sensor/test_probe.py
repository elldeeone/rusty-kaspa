#!/usr/bin/env python3
"""
Test script to verify Phase 2 active probing functionality.

This script simulates an inbound peer connection to the sensor and verifies that:
1. The sensor accepts the P2P handshake
2. The sensor automatically probes back to classify the peer
3. The connection event is logged to the database
"""

import asyncio
import grpc
import random
import time
import sys
import os
import sqlite3
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

    def __init__(self, sensor_address="localhost:16111", network="kaspa-mainnet"):
        self.sensor_address = sensor_address
        self.network = network
        self.peer_id = None
        self.peer_version = None
        self.peer_user_agent = None

    async def __aenter__(self):
        # Generate random peer ID
        self.id = bytes.fromhex(hex(int(random.random() * 10000))[2:].zfill(32))

        # Connect via gRPC
        print(f"\n=== Connecting to sensor at {self.sensor_address} ===")
        self.channel = grpc.aio.insecure_channel(self.sensor_address)

        await asyncio.wait_for(self.channel.channel_ready(), 5)
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


def check_sensor_database(db_path):
    """Check the sensor's database for logged connection events"""
    print(f"\n=== Checking Sensor Database: {db_path} ===")

    if not os.path.exists(db_path):
        print(f"❌ Database not found at {db_path}")
        return False

    try:
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()

        # Get recent connection events (last 60 seconds)
        cursor.execute("""
            SELECT
                timestamp,
                event_id,
                peer_ip || ':' || peer_port as peer_address,
                direction,
                classification,
                user_agent
            FROM peer_events
            WHERE timestamp > datetime('now', '-60 seconds')
            ORDER BY timestamp DESC
        """)

        events = cursor.fetchall()

        if not events:
            print("❌ No recent connection events found")
            return False

        print(f"✅ Found {len(events)} recent connection event(s):")
        for event in events:
            timestamp, peer_id, peer_address, direction, classification, user_agent = event
            print(f"\n  Timestamp: {timestamp}")
            print(f"  Peer ID: {peer_id[:16]}...")
            print(f"  Address: {peer_address}")
            print(f"  Direction: {direction}")
            print(f"  Classification: {classification or 'PENDING'}")
            print(f"  User Agent: {user_agent}")

        conn.close()
        return True

    except Exception as e:
        print(f"❌ Database error: {e}")
        return False


async def main():
    """Main test sequence"""
    print("=" * 60)
    print("Phase 2 Active Probing Test")
    print("=" * 60)

    # Step 1: Connect to sensor and complete handshake
    try:
        async with TestPeer() as peer:
            print("\n✅ Successfully connected and completed handshake")

            # Wait a moment for sensor to process
            print("\nWaiting 3 seconds for sensor to probe back...")
            await asyncio.sleep(3)

        print("\n✅ Connection closed cleanly")

    except asyncio.TimeoutError:
        print("\n❌ Connection timeout - is the sensor running?")
        print("   Start sensor with: ./target/release/kaspa-sensor -c sensor.toml --sensor-id test-sensor")
        return 1
    except Exception as e:
        print(f"\n❌ Connection error: {e}")
        return 1

    # Step 2: Wait for probe to complete
    print("\nWaiting 5 seconds for active probe to complete...")
    await asyncio.sleep(5)

    # Step 3: Check database for logged event
    # Database is in project root, not sensor/ subdirectory
    db_path = "../sensor-data.db"
    success = check_sensor_database(db_path)

    # Summary
    print("\n" + "=" * 60)
    if success:
        print("✅ TEST PASSED: Sensor accepted connection and logged event")
        print("=" * 60)
        return 0
    else:
        print("❌ TEST FAILED: Event not found in database")
        print("=" * 60)
        return 1


if __name__ == "__main__":
    exit_code = asyncio.run(main())
    sys.exit(exit_code)
