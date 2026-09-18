#!/usr/bin/env bash
# One-hour soak: every 60s, for the 5 busiest StonkFun curve tokens, compare our newest closed 1m candle
# against Raydium's kline for the same minute. Logs to soak.log; summary printed at the end.
cd "$(dirname "$0")"
PW=$(cat /root/apme-be/.pg_ro_pw)
Q() { docker exec apeme-pg psql -U apeme_ro -d apeme -tAc "$1"; }
END=$(( $(date +%s) + 3600 ))
: > soak.log
while [ "$(date +%s)" -lt "$END" ]; do
  NOW=$(date +%s); MIN=$(( NOW / 60 * 60 - 60 ))
  Q "select t.mint, t.curve_pool from tokens t join token_stats s on s.token_mint=t.mint where t.launchpad='stonkfun' and t.phase='curve' and t.curve_pool is not null order by s.vol_24h_usd desc limit 5" | while IFS='|' read -r MINT POOL; do
    OURS=$(Q "select c from candles_1m where token_mint='$MINT' and minute=$MIN")
    [ -z "$OURS" ] && continue
    RAY=$(curl -s -m 10 "https://launch-history-v1.raydium.io/kline?poolId=$POOL&interval=1m&limit=3" -H 'User-Agent: Mozilla/5.0 ape-indexer' | python3 -c "import sys,json
try:
  rows=json.load(sys.stdin)['data']['rows']
  print(next((r['c'] for r in rows if int(r['t'])==$MIN), ''))
except Exception: print('')")
    [ -z "$RAY" ] && continue
    DEV=$(python3 -c "o=float('$OURS'); r=float('$RAY'); print(f'{abs(o-r)/r*100:.4f}')")
    echo "$(date -u +%H:%M:%S) $MINT min=$MIN ours=$OURS raydium=$RAY dev%=$DEV" >> soak.log
  done
  sleep $(( 60 - ($(date +%s) % 60) ))
done
python3 - <<'PY'
import re
rows=[float(m.group(1)) for l in open('soak.log') for m in [re.search(r'dev%=([0-9.]+)',l)] if m]
rows.sort()
print(f"samples={len(rows)} exact(<0.01%)={sum(r<0.01 for r in rows)} max_dev%={rows[-1] if rows else 'n/a'} p95%={rows[int(len(rows)*0.95)] if rows else 'n/a'}")
PY
