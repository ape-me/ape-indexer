#!/bin/sh
# Unhealthy when the stream is down, the indexer is >750 slots behind, or its slot has not moved for 3 minutes.
# autoheal (in apme-ops compose) restarts the container when unhealthy. Replaces the old watchdog cron.
m=$(curl -sf -m 5 http://127.0.0.1:9464/metrics) || exit 1
get() { echo "$m" | awk -v k="$1" '$1==k {print int($2)}'; }
slot=$(get ape_indexer_slot); chain=$(get ape_chain_slot); up=$(get ape_stream_connected)
[ "${up:-0}" -eq 1 ] || exit 1
[ $((${chain:-0} - ${slot:-0})) -lt 750 ] || exit 1
now=$(date +%s); f=/tmp/hc.last
if [ -f "$f" ]; then read ls lt < "$f"; if [ "$slot" = "$ls" ]; then [ $((now - lt)) -lt 180 ] || exit 1; exit 0; fi; fi
echo "$slot $now" > "$f"
