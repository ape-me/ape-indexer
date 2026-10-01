DROP TABLE IF EXISTS sol_owed;
ALTER TABLE orders DROP COLUMN IF EXISTS rent_lamports, DROP COLUMN IF EXISTS sweep_lamports;
ALTER TABLE withdrawals DROP COLUMN IF EXISTS sweep_lamports;
