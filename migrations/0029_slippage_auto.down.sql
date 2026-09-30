ALTER TABLE user_settings ALTER COLUMN slippage_bps SET DEFAULT 100;
UPDATE user_settings SET slippage_bps = 100 WHERE slippage_bps = 0;
