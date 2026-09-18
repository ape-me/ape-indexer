-- Terminal data: short windows, ATH, socials, DexScreener paid status, holder analysis from our own tape.
ALTER TABLE token_stats
  ADD COLUMN IF NOT EXISTS vol_5m_usd   DOUBLE PRECISION NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS buys_5m      INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS sells_5m     INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS vol_1h_usd   DOUBLE PRECISION NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS buys_1h      INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS sells_1h     INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS change_1h    DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS ath_mcap_usd DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS holders      INTEGER,
  ADD COLUMN IF NOT EXISTS top10_pct    DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS dev_pct      DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS snipers_pct  DOUBLE PRECISION,
  ADD COLUMN IF NOT EXISTS holders_at   BIGINT;

ALTER TABLE tokens
  ADD COLUMN IF NOT EXISTS website        TEXT,
  ADD COLUMN IF NOT EXISTS twitter        TEXT,
  ADD COLUMN IF NOT EXISTS telegram       TEXT,
  ADD COLUMN IF NOT EXISTS socials_at     BIGINT,
  ADD COLUMN IF NOT EXISTS dex_paid       BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS dex_paid_at    BIGINT,
  ADD COLUMN IF NOT EXISTS dex_boosts     INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS dex_checked_at BIGINT;

-- column queries: curve tokens by progress, graduated by 1h volume, everything by creation
CREATE INDEX IF NOT EXISTS token_stats_vol_1h ON token_stats (vol_1h_usd DESC);
CREATE INDEX IF NOT EXISTS token_stats_progress ON token_stats (progress_pct DESC);
CREATE INDEX IF NOT EXISTS tokens_phase_created ON tokens (phase, created_at DESC);
CREATE INDEX IF NOT EXISTS trades_token_wallet ON trades (token_mint, wallet);
