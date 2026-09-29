-- In-app account deletion (App Store 5.1.1(v)): the row is anonymised and stamped, never dropped, so
-- swaps, orders and referrals keep their user_id.
ALTER TABLE users ADD COLUMN IF NOT EXISTS deleted_at bigint;
-- The API role deletes wallets and settings on account deletion; everything else it only anonymises.
GRANT DELETE ON wallets, user_settings TO apeme_ro;
