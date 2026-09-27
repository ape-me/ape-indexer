-- Three issuers can tokenize the same company (AAPLx, AAPL.US, AAPLon). `underlying` is the real-world ticker,
-- so the list can show one row per company (the deepest pool) while every mint stays tradeable by address.
-- Crypto and yield tokens live in the same table: they price and swap exactly like a stock does.
ALTER TABLE stocks ADD COLUMN IF NOT EXISTS underlying text;
CREATE INDEX IF NOT EXISTS stocks_underlying_idx ON stocks (underlying);
