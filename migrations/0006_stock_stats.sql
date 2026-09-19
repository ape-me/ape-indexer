-- Per-floor 24h rollup, refreshed every 60s by the indexer, so the API never aggregates trades per request.
CREATE TABLE IF NOT EXISTS stock_stats (
  mint          TEXT PRIMARY KEY REFERENCES stocks(mint),
  launched_24h  INTEGER NOT NULL DEFAULT 0,
  meme_vol_24h  DOUBLE PRECISION NOT NULL DEFAULT 0,   -- USD at trade time
  trades_24h    INTEGER NOT NULL DEFAULT 0,
  wallets_24h   INTEGER NOT NULL DEFAULT 0,
  heat          DOUBLE PRECISION NOT NULL DEFAULT 0,   -- launched*10 + wallets + vol/1000
  king_mint     TEXT,                                  -- top meme by 24h volume on this floor
  updated_at    BIGINT NOT NULL
);
GRANT SELECT ON stock_stats TO apeme_ro;
