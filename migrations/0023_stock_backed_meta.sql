-- Backed is the issuer of the xStocks, so it is the authority on their symbol, name and logo; the launchpad
-- quote list we discover mints from had two wrong (APPLX for AAPLx, BRKX for BRK.Bx) and uppercased the rest.
-- `halted` is Backed's own trading halt: an order on a halted stock cannot fill, so it must not be offered.
ALTER TABLE stocks ADD COLUMN IF NOT EXISTS halted boolean NOT NULL DEFAULT false;
