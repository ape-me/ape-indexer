-- SOL Jupiter hands a maker when an order closes is ours (we fronted the rent); it is swept back on the next
-- transaction the wallet signs with us. `rent_lamports` is what that order will hand back.
CREATE TABLE IF NOT EXISTS sol_owed (
  wallet     text PRIMARY KEY,
  lamports   bigint NOT NULL DEFAULT 0,
  updated_at bigint NOT NULL
);
ALTER TABLE orders ADD COLUMN IF NOT EXISTS rent_lamports bigint, ADD COLUMN IF NOT EXISTS sweep_lamports bigint NOT NULL DEFAULT 0;
ALTER TABLE withdrawals ADD COLUMN IF NOT EXISTS sweep_lamports bigint NOT NULL DEFAULT 0;
GRANT SELECT, INSERT, UPDATE, DELETE ON sol_owed TO apeme_ro;
