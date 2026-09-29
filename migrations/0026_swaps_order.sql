-- Swap API v2 `/order`: who paid gas (user | jupiter | apeme), the winning router and Jupiter's request id for `/execute`.
ALTER TABLE swaps
  ADD COLUMN IF NOT EXISTS payer text NOT NULL DEFAULT 'apeme',
  ADD COLUMN IF NOT EXISTS router text,
  ADD COLUMN IF NOT EXISTS jup_request_id text;
