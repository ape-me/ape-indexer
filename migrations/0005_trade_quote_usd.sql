-- USD price of the quote stock at the moment of the trade, so volume is "at trade time", not "at today's price".
ALTER TABLE trades ADD COLUMN IF NOT EXISTS quote_usd DOUBLE PRECISION;
-- backfill: nearest stock snapshot within 10 minutes, else the stock's current price
UPDATE trades t SET quote_usd = COALESCE(
  (SELECT sn.price_usd FROM stock_snapshots sn JOIN tokens k ON k.mint = t.token_mint
     WHERE sn.mint = k.quote_mint AND sn.ts BETWEEN t.block_time - 600 AND t.block_time + 600
     ORDER BY abs(sn.ts - t.block_time) LIMIT 1),
  (SELECT s.price_usd FROM tokens k JOIN stocks s ON s.mint = k.quote_mint WHERE k.mint = t.token_mint))
WHERE t.quote_usd IS NULL;
-- crypto pairs that Backpack lists next to equities: keep their floors, but they are not stocks
UPDATE stocks SET category = 'crypto' WHERE symbol IN ('ARB','CHIP','DOGE','INJ','LINK','PEPE','TAO','PEAQ','PONS','ROBOSTRATEGY','PSG','PENG');
