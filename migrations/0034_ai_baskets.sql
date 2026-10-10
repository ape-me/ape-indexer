-- Baskets an AI built from a user's idea: picks with their weights, kept for /v1/baskets/ai_xxx, the daily limit and tuning.
CREATE TABLE IF NOT EXISTS ai_baskets (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id),
  idea text NOT NULL,
  name text NOT NULL,
  tagline text,
  bear_case text,
  picks jsonb NOT NULL,
  model text NOT NULL,
  created_at bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS ai_baskets_user ON ai_baskets (user_id, created_at);
-- The mix a buy was split by ({ticker: percent}); null = the basket's own weights.
ALTER TABLE basket_orders ADD COLUMN IF NOT EXISTS weights jsonb;
GRANT SELECT, INSERT, UPDATE ON ai_baskets TO apeme_ro;
