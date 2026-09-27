-- Jupiter's escrow accounts cost rent at creation and refund to the maker, never to us, so the order carries it.
ALTER TABLE orders ADD COLUMN IF NOT EXISTS rent_usd DOUBLE PRECISION;
