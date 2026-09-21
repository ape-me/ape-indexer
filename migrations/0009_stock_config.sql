-- Operator overrides for the stock list, so hiding/re-categorising/tagging a stock is a row, not a deploy.
-- The indexer applies it on every list sync (10 min); the API applies excluded/tags immediately via the admin route.
CREATE TABLE IF NOT EXISTS stock_config (
  mint        text PRIMARY KEY,
  excluded    boolean NOT NULL DEFAULT false,
  category    text,                          -- override: preipo | stock | etf | crypto; NULL = derived
  tags        text[] NOT NULL DEFAULT '{}',  -- extra collection ids, e.g. {ai,defense}
  note        text,
  updated_at  bigint NOT NULL
);
ALTER TABLE stocks ADD COLUMN IF NOT EXISTS excluded boolean NOT NULL DEFAULT false;
ALTER TABLE stocks ADD COLUMN IF NOT EXISTS tags text[] NOT NULL DEFAULT '{}';
GRANT SELECT ON stock_config TO apeme_ro;
-- The API connects as apeme_ro; the admin route is its only write path.
GRANT INSERT, UPDATE ON stock_config TO apeme_ro;
GRANT UPDATE (excluded, tags, category) ON stocks TO apeme_ro;
