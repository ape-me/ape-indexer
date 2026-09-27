-- Jupiter writes the deadline onto the order account but does not act on it: an expired order keeps the
-- maker's funds until they sign a cancel. We store the deadline so the app can tell them to.
ALTER TABLE orders ADD COLUMN IF NOT EXISTS expires_at bigint;
