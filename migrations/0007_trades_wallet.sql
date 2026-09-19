-- wallet lookups for the portfolio screen (holdings cost basis, recent activity)
CREATE INDEX IF NOT EXISTS trades_wallet_time ON trades (wallet, block_time DESC);
