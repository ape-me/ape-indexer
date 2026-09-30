-- News rows about the market itself (rates, tariffs, the indexes) carry no stock; scope tells them apart.
ALTER TABLE news ADD COLUMN IF NOT EXISTS scope text NOT NULL DEFAULT 'stock';
CREATE INDEX IF NOT EXISTS news_scope_published_idx ON news (scope, published_at DESC);
