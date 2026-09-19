-- ApeMe phase 0 schema. Amounts are raw NUMERIC (u64 range), prices DOUBLE, times unix seconds.

CREATE TABLE stocks (
  mint        TEXT PRIMARY KEY,
  symbol      TEXT NOT NULL,
  name        TEXT NOT NULL,
  issuer      TEXT NOT NULL,            -- xstocks | backpack | prestocks
  category    TEXT NOT NULL,            -- stock | etf | preipo | index
  decimals    SMALLINT NOT NULL,
  logo        TEXT,
  price_usd   DOUBLE PRECISION,
  change_24h  DOUBLE PRECISION,
  updated_at  BIGINT
);

CREATE TABLE tokens (
  mint        TEXT PRIMARY KEY,
  symbol      TEXT,
  name        TEXT,
  image       TEXT,
  uri         TEXT,
  quote_mint  TEXT NOT NULL REFERENCES stocks(mint),
  launchpad   TEXT NOT NULL,            -- stonkfun | pumpfun | dbc
  creator     TEXT,
  decimals    SMALLINT NOT NULL DEFAULT 6,
  supply      NUMERIC,
  phase       TEXT NOT NULL DEFAULT 'curve',   -- curve | graduated
  curve_pool  TEXT,
  amm_pool    TEXT,
  tax_bps     INTEGER NOT NULL DEFAULT 0,
  created_at  BIGINT NOT NULL,
  source      TEXT NOT NULL DEFAULT 'stream'   -- stream | backfill
);
CREATE INDEX tokens_quote_created ON tokens (quote_mint, created_at DESC);

CREATE TABLE pools (
  pool        TEXT PRIMARY KEY,
  token_mint  TEXT NOT NULL REFERENCES tokens(mint),
  program     TEXT NOT NULL,            -- launchlab | cpmm | pumpfun | pumpswap | dbc | damm2
  kind        TEXT NOT NULL,            -- curve | amm
  base_vault  TEXT,
  quote_vault TEXT,
  created_at  BIGINT NOT NULL,
  migrated_to TEXT
);
CREATE INDEX pools_token ON pools (token_mint);

CREATE TABLE trades (
  signature   TEXT NOT NULL,
  ix_index    SMALLINT NOT NULL,
  slot        BIGINT NOT NULL,
  block_time  BIGINT NOT NULL,
  pool        TEXT NOT NULL,
  token_mint  TEXT NOT NULL,
  wallet      TEXT NOT NULL,
  side        TEXT NOT NULL,            -- buy | sell
  base_raw    NUMERIC NOT NULL,
  quote_raw   NUMERIC NOT NULL,
  price_quote DOUBLE PRECISION NOT NULL,
  PRIMARY KEY (signature, ix_index)
);
CREATE INDEX trades_token_time ON trades (token_mint, block_time DESC);

CREATE TABLE candles_1m (
  token_mint  TEXT NOT NULL,
  minute      BIGINT NOT NULL,          -- unix seconds, floored to 60
  o DOUBLE PRECISION NOT NULL, h DOUBLE PRECISION NOT NULL,
  l DOUBLE PRECISION NOT NULL, c DOUBLE PRECISION NOT NULL,
  vol_quote   DOUBLE PRECISION NOT NULL DEFAULT 0,
  n           INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (token_mint, minute)
);

CREATE TABLE token_stats (
  token_mint    TEXT PRIMARY KEY REFERENCES tokens(mint),
  price_quote   DOUBLE PRECISION,
  price_usd     DOUBLE PRECISION,
  mcap_usd      DOUBLE PRECISION,
  vol_24h_usd   DOUBLE PRECISION NOT NULL DEFAULT 0,
  buys_24h      INTEGER NOT NULL DEFAULT 0,
  sells_24h     INTEGER NOT NULL DEFAULT 0,
  change_24h    DOUBLE PRECISION,
  progress_pct  DOUBLE PRECISION,
  last_trade_at BIGINT,
  updated_at    BIGINT
);

CREATE TABLE cursor (
  program        TEXT PRIMARY KEY,
  last_slot      BIGINT NOT NULL,
  last_signature TEXT,
  updated_at     BIGINT NOT NULL
);
