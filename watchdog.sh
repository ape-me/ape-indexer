#!/bin/bash
# Restarts ape-indexer if its cursor has not advanced for 3 minutes. Runs from cron every minute.
set -a; . /root/ape-indexer/.env; set +a
lag=$(psql "$DATABASE_URL" -Atc "select extract(epoch from now())::bigint - updated_at from cursor where program='stream'" 2>/dev/null)
if [ -z "$lag" ] || [ "$lag" -gt 180 ]; then
  echo "$(date -u +%FT%TZ) cursor lag=${lag:-none}s, restarting" >> /root/ape-indexer/watchdog.log
  systemctl restart ape-indexer
fi
