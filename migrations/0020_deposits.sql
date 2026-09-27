-- USDC in and out of a user's wallet. Read live from the chain on every wallet view, kept here so activity
-- can be paged and date-filtered alongside trades instead of being capped at the last few signatures.
CREATE TABLE IF NOT EXISTS deposits (
  signature  text PRIMARY KEY,
  wallet     text NOT NULL,
  ts         bigint NOT NULL,
  direction  text NOT NULL CHECK (direction IN ('in', 'out')),
  amount     double precision NOT NULL,
  from_addr  text
);
CREATE INDEX IF NOT EXISTS deposits_wallet_ts ON deposits (wallet, ts DESC);
