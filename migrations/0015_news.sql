CREATE TABLE IF NOT EXISTS news (
  id            text PRIMARY KEY,
  title         text NOT NULL,
  summary       text,
  source        text,
  url           text NOT NULL UNIQUE,
  image         text,
  published_at  bigint NOT NULL,
  fetched_at    bigint NOT NULL,
  impact        smallint,
  direction     text,
  confidence    real,
  junk          boolean NOT NULL DEFAULT false
);
CREATE TABLE IF NOT EXISTS news_stocks (
  news_id   text NOT NULL REFERENCES news(id) ON DELETE CASCADE,
  mint      text NOT NULL,
  relevance real NOT NULL DEFAULT 1,
  PRIMARY KEY (news_id, mint)
);
CREATE INDEX IF NOT EXISTS news_stocks_mint_idx ON news_stocks (mint);
CREATE INDEX IF NOT EXISTS news_published_idx ON news (published_at DESC);
GRANT SELECT, INSERT, UPDATE, DELETE ON news, news_stocks TO apeme_ro;
