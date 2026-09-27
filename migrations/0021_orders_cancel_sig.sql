-- `signature` is the transaction that placed the order; a cancel is its own transaction and its own link.
ALTER TABLE orders ADD COLUMN IF NOT EXISTS cancel_signature text;
