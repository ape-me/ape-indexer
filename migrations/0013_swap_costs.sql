ALTER TABLE swaps ADD COLUMN IF NOT EXISTS swap_usd numeric, ADD COLUMN IF NOT EXISTS rent_usd numeric, ADD COLUMN IF NOT EXISTS issuer_fee_usd numeric;
-- Backfill the four pre-migration trades from their on-chain USDC deltas.
UPDATE swaps SET in_usd = 1.00, swap_usd = 0.99, rent_usd = 0,    issuer_fee_usd = 0 WHERE id = 'a01947d4-d6e9-4832-8c65-e32d4d859939';
UPDATE swaps SET in_usd = 1.25, swap_usd = 0.99, rent_usd = 0.25, issuer_fee_usd = 0 WHERE id = 'bedc8752-79c6-47fb-b564-22630e8322fb';
UPDATE swaps SET in_usd = 1.00, swap_usd = 0.75, rent_usd = 0.24, issuer_fee_usd = 0.0075 WHERE id = '71d04036-c526-498f-9b51-bee951067953';
UPDATE swaps SET in_usd = 1.00, swap_usd = 0.75, rent_usd = 0.24, issuer_fee_usd = 0.0075 WHERE id = '51cf38c5-08a4-4b8f-8ee9-e0070c2ce27a';
