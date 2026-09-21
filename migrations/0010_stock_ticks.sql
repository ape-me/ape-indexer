-- 5-second stock price ticks for the live hero price and short chart ranges (5m/15m/1h). 24h retention.
CREATE TABLE IF NOT EXISTS stock_ticks (mint text NOT NULL REFERENCES stocks(mint), ts bigint NOT NULL, price_usd double precision NOT NULL, PRIMARY KEY (mint, ts));
GRANT SELECT ON stock_ticks TO apeme_ro;
