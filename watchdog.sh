#!/bin/bash
# Restarts ape-indexer if the cursor stalls (>3 min) or falls >5 min behind the chain. Cron, every minute.
set -a; . /root/ape-indexer/.env; set +a
now=$(date +%s)
read cur_slot cur_upd < <(psql "$DATABASE_URL" -Atc "select last_slot, updated_at from cursor where program='stream'" | tr '|' ' ')
chain=$(curl -s -m 8 "$RPC_URL" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"confirmed"}]}' | python3 -c "import json,sys; print(json.load(sys.stdin)['result'])" 2>/dev/null)
stall=$(( now - ${cur_upd:-0} ))
behind=$(( ${chain:-0} - ${cur_slot:-0} ))
echo "$(date -u +%FT%TZ) stall=${stall}s behind=${behind}slots" >> /root/ape-indexer/watchdog.status
if [ "$stall" -gt 180 ]; then echo "$(date -u +%FT%TZ) stalled ${stall}s, restarting" >> /root/ape-indexer/watchdog.log; systemctl restart ape-indexer; exit; fi
if [ -n "$chain" ] && [ "$behind" -gt 750 ]; then echo "$(date -u +%FT%TZ) behind chain by ${behind} slots, restarting" >> /root/ape-indexer/watchdog.log; systemctl restart ape-indexer; fi
