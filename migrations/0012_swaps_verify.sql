-- Swap quote bookkeeping: message hash so submit can prove the signed tx is the one we built, blockhash expiry, fee in raw USDC.
ALTER TABLE swaps ADD COLUMN IF NOT EXISTS msg_hash text;
ALTER TABLE swaps ADD COLUMN IF NOT EXISTS last_valid_block_height bigint;
ALTER TABLE swaps ADD COLUMN IF NOT EXISTS fee_raw numeric;
ALTER TABLE swaps ADD COLUMN IF NOT EXISTS symbol text;
