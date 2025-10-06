-- Migration: Add peer_id support to peer_events table
-- Version: 0.2.5
-- Date: 2025-10-06

-- Add peer_id column to existing peer_events table
ALTER TABLE peer_events ADD COLUMN IF NOT EXISTS peer_id TEXT;

-- Create index on peer_id for faster queries
CREATE INDEX IF NOT EXISTS idx_peer_events_peer_id ON peer_events(peer_id) WHERE peer_id IS NOT NULL;

-- Drop and recreate the peer_persistence materialized view with peer_id support
DROP MATERIALIZED VIEW IF EXISTS peer_persistence CASCADE;

CREATE MATERIALIZED VIEW peer_persistence AS
SELECT
    peer_address,
    array_agg(DISTINCT peer_id) FILTER (WHERE peer_id IS NOT NULL) as peer_ids,
    COUNT(DISTINCT peer_id) FILTER (WHERE peer_id IS NOT NULL) as unique_peer_ids,
    MIN(created_at) as first_seen,
    MAX(created_at) as last_seen,
    COUNT(*) as total_connections,
    COUNT(DISTINCT sensor_id) as sensors_seen_by,
    array_agg(DISTINCT sensor_id) as sensor_list,
    MAX(classification) as latest_classification,
    MAX((metadata::text)::jsonb->>'user_agent') as latest_user_agent
FROM peer_events
GROUP BY peer_address;

-- Recreate indexes on materialized view
CREATE INDEX IF NOT EXISTS idx_peer_persistence_address ON peer_persistence(peer_address);
CREATE INDEX IF NOT EXISTS idx_peer_persistence_connections ON peer_persistence(total_connections DESC);
CREATE INDEX IF NOT EXISTS idx_peer_persistence_first_seen ON peer_persistence(first_seen DESC);
CREATE INDEX IF NOT EXISTS idx_peer_persistence_last_seen ON peer_persistence(last_seen DESC);
