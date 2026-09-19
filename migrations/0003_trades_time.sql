-- time-range scans for the Product dashboard (volume/trades per hour)
CREATE INDEX CONCURRENTLY IF NOT EXISTS trades_time ON trades (block_time);
