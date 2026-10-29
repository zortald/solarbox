#!/bin/sh
# Replace config.json on a running solarbox (validated server-side, live).
#   SB_ADMIN=<password> ./deploy/push-config.sh [file]   # default deploy/site-config.json
set -e
[ -f "$(dirname "$0")/local.env" ] && . "$(dirname "$0")/local.env"
URL=${SB_URL:?set SB_URL (deploy/local.env)}
: "${SB_ADMIN:?set SB_ADMIN to the solarbox admin password}"
F=${1:-$(dirname "$0")/site-config.json}
curl -fsS -X PUT -H "X-Admin: $SB_ADMIN" -H 'Content-Type: application/json' --data-binary @"$F" "$URL/api/config"; echo
curl -fsS "$URL/api/config" | python3 -c 'import json,sys;d=json.load(sys.stdin);print("boards:",[b["name"] for b in d["boards"]],"| load_error:",d.get("load_error"))'
