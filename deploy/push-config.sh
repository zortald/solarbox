#!/bin/sh
# Replace config.json on a running solarbox (validated server-side, live).
#   SB_ADMIN=<emergency password> ./deploy/push-config.sh [file]   # default deploy/site-config.json (local, not committed; see site-config.example.json)
set -e
[ -f "$(dirname "$0")/local.env" ] && . "$(dirname "$0")/local.env"
: "${SB_ADMIN:?set SB_ADMIN to the solarbox emergency password}"
. "$(dirname "$0")/sb-https.sh"
URL=$SB_HTTPS
F=${1:-$(dirname "$0")/site-config.json}
curl -fsS $SB_TLS -X PUT -H "X-Admin: $SB_ADMIN" -H 'Content-Type: application/json' --data-binary @"$F" "$URL/api/config"; echo
curl -fsS $SB_TLS "$URL/api/config" | python3 -c 'import json,sys;d=json.load(sys.stdin);print("boards:",[b["name"] for b in d["boards"]],"| load_error:",d.get("load_error"))'
