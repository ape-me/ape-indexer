-- Operator knobs that change without a deploy. Values are text; readers coerce and keep a fallback.
CREATE TABLE IF NOT EXISTS app_config (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at BIGINT NOT NULL
);
GRANT SELECT, INSERT, UPDATE, DELETE ON app_config TO apeme_ro;
