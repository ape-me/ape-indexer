-- Accounts: Privy users, their wallets, trading settings, swaps we build, sponsored rent, watchlist,
-- invite gate and referral rewards. First tables the API writes (same role as stock_config, see 0009).
-- The indexer never touches these. Mints are text joins, not FKs, because stocks/tokens can be reloaded.

CREATE TABLE IF NOT EXISTS users (
  id             text PRIMARY KEY,               -- Privy DID (did:privy:...)
  handle         text UNIQUE,
  avatar_url     text,
  email_hash     text,
  referral_code  text UNIQUE NOT NULL,           -- 6 chars, doubles as the user's invite code
  invited_by     text REFERENCES users(id),
  activated_at   bigint,                         -- NULL = invite_required
  created_at     bigint NOT NULL,
  last_seen_at   bigint NOT NULL
);

CREATE TABLE IF NOT EXISTS wallets (
  address     text PRIMARY KEY,
  user_id     text NOT NULL REFERENCES users(id),
  chain       text NOT NULL DEFAULT 'solana',    -- solana | evm
  hd_index    int  NOT NULL DEFAULT 0,
  label       text NOT NULL DEFAULT 'Main',
  is_default  boolean NOT NULL DEFAULT false,
  created_at  bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS wallets_user ON wallets (user_id);

CREATE TABLE IF NOT EXISTS user_settings (
  user_id               text PRIMARY KEY REFERENCES users(id),
  slippage_bps          int NOT NULL DEFAULT 100,
  quick_buy_usd         int[] NOT NULL DEFAULT '{10,25,50,100}',
  quick_sell_pct        int[] NOT NULL DEFAULT '{25,50,100}',
  priority              text NOT NULL DEFAULT 'normal',   -- normal | fast | turbo
  confirm_before_trade  boolean NOT NULL DEFAULT true,
  hide_dust             boolean NOT NULL DEFAULT false,
  updated_at            bigint NOT NULL
);

CREATE TABLE IF NOT EXISTS swaps (
  id                uuid PRIMARY KEY,
  user_id           text NOT NULL REFERENCES users(id),
  wallet            text NOT NULL,
  chain             text NOT NULL DEFAULT 'solana',
  side              text NOT NULL,                 -- buy | sell
  input_mint        text NOT NULL,
  output_mint       text NOT NULL,
  in_raw            numeric NOT NULL,
  out_raw           numeric,
  min_out_raw       numeric,
  in_usd            double precision,
  out_usd           double precision,
  fee_bps           int NOT NULL DEFAULT 0,
  fee_usd           double precision,
  price_impact_pct  double precision,
  premium_pct       double precision,              -- frozen at quote time
  priority          text NOT NULL DEFAULT 'normal',
  gas_lamports      bigint NOT NULL DEFAULT 0,      -- what we paid
  rent_lamports     bigint NOT NULL DEFAULT 0,
  signature         text UNIQUE,
  slot              bigint,
  status            text NOT NULL DEFAULT 'quoted', -- quoted | submitted | confirmed | failed
  error             text,
  created_at        bigint NOT NULL,
  submitted_at      bigint,
  confirmed_at      bigint
);
CREATE INDEX IF NOT EXISTS swaps_user_time   ON swaps (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS swaps_wallet_time ON swaps (wallet, created_at DESC);

CREATE TABLE IF NOT EXISTS sponsored_rent (
  user_id   text NOT NULL REFERENCES users(id),
  mint      text NOT NULL,
  lamports  bigint NOT NULL,
  paid_at   bigint NOT NULL,
  PRIMARY KEY (user_id, mint)
);

CREATE TABLE IF NOT EXISTS watchlist (
  user_id   text NOT NULL REFERENCES users(id),
  mint      text NOT NULL,
  added_at  bigint NOT NULL,
  PRIMARY KEY (user_id, mint)
);

CREATE TABLE IF NOT EXISTS invite_codes (
  code           text PRIMARY KEY,
  kind           text NOT NULL,                   -- admin | user
  owner_user_id  text REFERENCES users(id),       -- NULL for admin codes
  max_uses       int NOT NULL,
  uses           int NOT NULL DEFAULT 0,
  label          text,
  expires_at     bigint,
  created_at     bigint NOT NULL
);

CREATE TABLE IF NOT EXISTS referrals (
  referee_user_id   text PRIMARY KEY REFERENCES users(id),
  referrer_user_id  text NOT NULL REFERENCES users(id),
  code              text NOT NULL REFERENCES invite_codes(code),
  created_at        bigint NOT NULL,
  CHECK (referee_user_id <> referrer_user_id)
);
CREATE INDEX IF NOT EXISTS referrals_referrer ON referrals (referrer_user_id);

CREATE TABLE IF NOT EXISTS referral_earnings (
  id                uuid PRIMARY KEY,
  referrer_user_id  text NOT NULL REFERENCES users(id),
  referee_user_id   text NOT NULL REFERENCES users(id),
  swap_id           uuid UNIQUE NOT NULL REFERENCES swaps(id),
  amount_usd        double precision NOT NULL,
  status            text NOT NULL DEFAULT 'accrued',  -- accrued | paid
  paid_signature    text,
  created_at        bigint NOT NULL,
  paid_at           bigint
);
CREATE INDEX IF NOT EXISTS referral_earnings_referrer ON referral_earnings (referrer_user_id, status);

-- The API role writes these; still no DELETE anywhere.
GRANT SELECT, INSERT, UPDATE ON users, wallets, user_settings, swaps, sponsored_rent, watchlist, invite_codes, referrals, referral_earnings TO apeme_ro;
GRANT DELETE ON watchlist TO apeme_ro;   -- unstar is a real delete
