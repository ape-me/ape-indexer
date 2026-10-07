-- Rebalancing is "coming soon": every position carries the switch, off until the feature ships.
ALTER TABLE basket_orders ADD COLUMN IF NOT EXISTS rebalance boolean NOT NULL DEFAULT false;
