-- Stock-level market data (the tokenized stock itself, not the memes on it). Live values on stocks,
-- a 60s history in stock_snapshots for the Stocks dashboard. PreStocks mark/valuation from their API.
ALTER TABLE stocks
  ADD COLUMN IF NOT EXISTS liquidity_usd     DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS vol_24h_usd       DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS vol_1h_usd        DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS buys_24h          INTEGER,
  ADD COLUMN IF NOT EXISTS sells_24h         INTEGER,
  ADD COLUMN IF NOT EXISTS mcap_usd          DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS change_1h         DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS mark_usd          DOUBLE PRECISION,   -- PreStocks: price of the underlying (their mark)
  ADD COLUMN IF NOT EXISTS premium_pct       DOUBLE PRECISION,   -- PreStocks: token price vs mark, +5 = trades 5% above
  ADD COLUMN IF NOT EXISTS mark_valuation    DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS implied_valuation DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS supply            DOUBLE PRECISION;

CREATE TABLE IF NOT EXISTS stock_snapshots (
  mint              TEXT NOT NULL REFERENCES stocks(mint),
  ts                BIGINT NOT NULL,
  price_usd         DOUBLE PRECISION,
  liquidity_usd     DOUBLE PRECISION,
  vol_24h_usd       DOUBLE PRECISION,
  vol_1h_usd        DOUBLE PRECISION,
  buys_24h          INTEGER,
  sells_24h         INTEGER,
  mcap_usd          DOUBLE PRECISION,
  mark_usd          DOUBLE PRECISION,
  premium_pct       DOUBLE PRECISION,
  mark_valuation    DOUBLE PRECISION,
  implied_valuation DOUBLE PRECISION,
  supply            DOUBLE PRECISION,
  PRIMARY KEY (mint, ts)
);
CREATE INDEX IF NOT EXISTS stock_snapshots_ts ON stock_snapshots (ts);
GRANT SELECT ON stock_snapshots TO apeme_ro;
