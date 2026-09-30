DROP INDEX IF EXISTS news_scope_published_idx;
ALTER TABLE news DROP COLUMN IF EXISTS scope;
