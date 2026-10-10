-- AI baskets start as drafts (null) and are kept once the author invests: "Your ideas" lists only those.
ALTER TABLE ai_baskets ADD COLUMN IF NOT EXISTS saved_at bigint;
