-- Withdrawals: a user-signed transfer of USDC, SOL or a holding to any wallet, gas and recipient rent paid by us.
CREATE TABLE IF NOT EXISTS withdrawals (
  id            text PRIMARY KEY,
  user_id       text NOT NULL REFERENCES users(id),
  wallet        text NOT NULL,
  to_address    text NOT NULL,
  mint          text NOT NULL,
  symbol        text,
  amount_raw    numeric NOT NULL,
  decimals      int NOT NULL,
  usd           numeric,
  rent_lamports bigint NOT NULL DEFAULT 0,
  msg_hash      text NOT NULL,
  signature     text,
  status        text NOT NULL DEFAULT 'quoted', -- quoted | submitted | confirmed | failed
  error         text,
  created_at    bigint NOT NULL,
  submitted_at  bigint,
  confirmed_at  bigint
);
CREATE INDEX IF NOT EXISTS withdrawals_user_time ON withdrawals (user_id, created_at DESC);
GRANT SELECT, INSERT, UPDATE ON withdrawals TO apeme_ro;
