-- Limit orders. Jupiter's Trigger program holds the escrow and their keepers fill it; this table is our
-- side of it: whose order it is, what it cost, and enough to show it before the chain confirms.
CREATE TABLE IF NOT EXISTS orders (
  id            TEXT PRIMARY KEY,           -- Jupiter's order account
  user_id       TEXT NOT NULL,
  wallet        TEXT NOT NULL,
  mint          TEXT NOT NULL,              -- the stock, whichever side it sits on
  symbol        TEXT,
  side          TEXT NOT NULL,              -- buy | sell
  input_mint    TEXT NOT NULL,
  output_mint   TEXT NOT NULL,
  making_raw    TEXT NOT NULL,
  taking_raw    TEXT NOT NULL,
  making_usd    DOUBLE PRECISION,
  trigger_usd   DOUBLE PRECISION,           -- price per token the order waits for
  status        TEXT NOT NULL,              -- quoted | open | filled | cancelled | failed
  request_id    TEXT,                       -- Jupiter's, while it is being signed
  msg_hash      TEXT,
  signature     TEXT,
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL,
  filled_at     BIGINT,
  fill_usd      DOUBLE PRECISION,
  fee_usd       DOUBLE PRECISION,
  error         TEXT
);
CREATE INDEX IF NOT EXISTS orders_user_idx ON orders (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS orders_open_idx ON orders (wallet) WHERE status IN ('quoted', 'open');

GRANT SELECT, INSERT, UPDATE ON orders TO apeme_ro;
