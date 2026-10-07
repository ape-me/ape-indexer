-- Ready-made baskets: one order per "invest $50 in Mag 7" or "sell Mag 7"; each stock in it is a normal swaps row.
CREATE TABLE IF NOT EXISTS basket_orders (
  id          uuid PRIMARY KEY,
  user_id     text NOT NULL REFERENCES users(id),
  wallet      text NOT NULL,
  basket_id   text NOT NULL,
  side        text NOT NULL,                  -- buy | sell
  amount_usd  double precision NOT NULL,
  status      text NOT NULL DEFAULT 'quoted', -- quoted | done | partial | failed
  created_at  bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS basket_orders_user ON basket_orders (user_id, basket_id);

ALTER TABLE swaps ADD COLUMN IF NOT EXISTS basket_order_id uuid REFERENCES basket_orders(id);
CREATE INDEX IF NOT EXISTS swaps_basket_order ON swaps (basket_order_id) WHERE basket_order_id IS NOT NULL;

-- Daily closes of the real stocks behind each basket (UTC day start), for the 1-year return and chart.
CREATE TABLE IF NOT EXISTS daily_closes (
  ticker text NOT NULL,
  ts     bigint NOT NULL,
  close  double precision NOT NULL,
  PRIMARY KEY (ticker, ts)
);

GRANT SELECT, INSERT, UPDATE ON basket_orders, daily_closes TO apeme_ro;
