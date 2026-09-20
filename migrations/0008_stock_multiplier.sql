-- xStocks/Backpack pay dividends and do splits by changing the Token-2022 scaled-UI multiplier, not by moving tokens.
-- 1 raw unit = multiplier displayed units. USD per raw unit = price_usd * multiplier.
ALTER TABLE stocks ADD COLUMN IF NOT EXISTS multiplier double precision NOT NULL DEFAULT 1;
