-- Quote expiry was a flat 45s recomputed from created_at at submit time, identical for pooled and RFQ (Ondo)
-- routes even though RFQ maker quotes go stale faster. Store the real expiry computed at quote time instead.
ALTER TABLE swaps ADD COLUMN IF NOT EXISTS expires_at bigint;
