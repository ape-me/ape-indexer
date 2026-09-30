-- Slippage 0 = let Jupiter size it per token. The fixed 100 bps default failed every sell on fee-on-transfer names.
ALTER TABLE user_settings ALTER COLUMN slippage_bps SET DEFAULT 0;
UPDATE user_settings SET slippage_bps = 0 WHERE slippage_bps = 100;
