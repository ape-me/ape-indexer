-- The same story reaches us through several outlets under different URLs, so the headline itself is the key.
ALTER TABLE news ADD COLUMN IF NOT EXISTS title_key TEXT;
UPDATE news SET title_key = lower(regexp_replace(title, '[^a-zA-Z0-9]+', '', 'g')) WHERE title_key IS NULL;
CREATE INDEX IF NOT EXISTS news_title_key_idx ON news (title_key);
