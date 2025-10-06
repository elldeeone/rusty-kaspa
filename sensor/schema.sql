-- PostgreSQL Schema for Kaspa Network Sensor
-- This schema stores peer events from multiple sensors for centralized analysis

-- Main table for peer connection events
CREATE TABLE IF NOT EXISTS peer_events (
    id BIGSERIAL PRIMARY KEY,
    sensor_id TEXT NOT NULL,
    peer_address TEXT NOT NULL,
    event_type TEXT NOT NULL,
    classification TEXT,
    timestamp BIGINT NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP DEFAULT NOW()
);

-- Indexes for fast queries
CREATE INDEX IF NOT EXISTS idx_peer_events_sensor_id ON peer_events(sensor_id);
CREATE INDEX IF NOT EXISTS idx_peer_events_timestamp ON peer_events(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_peer_events_classification ON peer_events(classification) WHERE classification IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_peer_events_created_at ON peer_events(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_peer_events_peer_address ON peer_events(peer_address);

-- Table for sensor metadata (tracks all sensors writing to this database)
CREATE TABLE IF NOT EXISTS sensor_metadata (
    sensor_id TEXT PRIMARY KEY,
    description TEXT,
    location TEXT,              -- Deployment location (e.g., "hetzner-de", "aws-us-east-1")
    environment TEXT,            -- Environment (e.g., "production", "development")
    ip_address INET,             -- Sensor's public IP address
    asn INTEGER,                 -- Sensor's ASN
    first_seen TIMESTAMP DEFAULT NOW(),
    last_seen TIMESTAMP DEFAULT NOW(),
    total_events BIGINT DEFAULT 0
);

-- Index for sensor lookups
CREATE INDEX IF NOT EXISTS idx_sensor_metadata_last_seen ON sensor_metadata(last_seen DESC);

-- Optional: Materialized view for classification statistics
CREATE MATERIALIZED VIEW IF NOT EXISTS classification_stats AS
SELECT
    sensor_id,
    classification,
    COUNT(*) as count,
    MIN(timestamp) as first_seen,
    MAX(timestamp) as last_seen
FROM peer_events
WHERE classification IS NOT NULL
GROUP BY sensor_id, classification;

CREATE INDEX IF NOT EXISTS idx_classification_stats_sensor ON classification_stats(sensor_id);

-- Function to refresh classification stats (call periodically)
CREATE OR REPLACE FUNCTION refresh_classification_stats()
RETURNS void AS $$
BEGIN
    REFRESH MATERIALIZED VIEW CONCURRENTLY classification_stats;
END;
$$ LANGUAGE plpgsql;

-- Materialized view for peer persistence tracking
-- Tracks first/last seen and connection count for each peer across all sensors
CREATE MATERIALIZED VIEW IF NOT EXISTS peer_persistence AS
SELECT
    peer_address,
    MIN(created_at) as first_seen,
    MAX(created_at) as last_seen,
    COUNT(*) as total_connections,
    COUNT(DISTINCT sensor_id) as sensors_seen_by,
    array_agg(DISTINCT sensor_id) as sensor_list,
    MAX(classification) as latest_classification,
    MAX((metadata::text)::jsonb->>'user_agent') as latest_user_agent
FROM peer_events
GROUP BY peer_address;

-- Indexes for peer persistence queries
CREATE INDEX IF NOT EXISTS idx_peer_persistence_address ON peer_persistence(peer_address);
CREATE INDEX IF NOT EXISTS idx_peer_persistence_connections ON peer_persistence(total_connections DESC);
CREATE INDEX IF NOT EXISTS idx_peer_persistence_first_seen ON peer_persistence(first_seen DESC);
CREATE INDEX IF NOT EXISTS idx_peer_persistence_last_seen ON peer_persistence(last_seen DESC);

-- Function to refresh peer persistence view
CREATE OR REPLACE FUNCTION refresh_peer_persistence()
RETURNS void AS $$
BEGIN
    REFRESH MATERIALIZED VIEW CONCURRENTLY peer_persistence;
END;
$$ LANGUAGE plpgsql;

-- Example queries for analysis:
--
-- 1. Get total events per sensor:
-- SELECT sensor_id, COUNT(*) as events FROM peer_events GROUP BY sensor_id;
--
-- 2. Get classification breakdown:
-- SELECT sensor_id, classification, COUNT(*) FROM peer_events
-- WHERE classification IS NOT NULL GROUP BY sensor_id, classification;
--
-- 3. Get hourly discovery rate:
-- SELECT
--   DATE_TRUNC('hour', TO_TIMESTAMP(timestamp)) as hour,
--   COUNT(*) as discoveries
-- FROM peer_events
-- WHERE event_type = 'discovered'
-- GROUP BY hour
-- ORDER BY hour DESC;
--
-- 4. Get unique peers discovered across all sensors:
-- SELECT COUNT(DISTINCT peer_address) FROM peer_events;
--
-- 5. Get public vs private peer ratio:
-- SELECT
--   classification,
--   COUNT(*) as count,
--   ROUND(100.0 * COUNT(*) / SUM(COUNT(*)) OVER (), 2) as percentage
-- FROM peer_events
-- WHERE classification IN ('public', 'private')
-- GROUP BY classification;

-- Grant permissions to sensor_writer user
GRANT INSERT, UPDATE, SELECT ON peer_events TO sensor_writer;
GRANT INSERT, UPDATE, SELECT ON sensor_metadata TO sensor_writer;
GRANT SELECT ON classification_stats TO sensor_writer;
GRANT SELECT ON peer_persistence TO sensor_writer;
