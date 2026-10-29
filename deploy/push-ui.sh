#!/bin/sh
# Push dashboard files to a running solarbox — live, no rebuild, no restart.
#   SB_ADMIN=<password> ./deploy/push-ui.sh                 # all of server/static
#   SB_ADMIN=<password> ./deploy/push-ui.sh app.css flow.js # only these
#   SB_ADMIN=<password> ./deploy/push-ui.sh --reset flow.js # back to the embedded copy
# SB_URL comes from deploy/local.env (or the environment).
set -e
[ -f "$(dirname "$0")/local.env" ] && . "$(dirname "$0")/local.env"
URL=${SB_URL:?set SB_URL (deploy/local.env)}
: "${SB_ADMIN:?set SB_ADMIN to the solarbox admin password}"
cd "$(dirname "$0")/../server/static"
reset=0
[ "$1" = "--reset" ] && { reset=1; shift; }
files=${*:-$(ls)}
for f in $files; do
  f=$(basename "$f")
  if [ $reset = 1 ]; then
    curl -fsS -X DELETE -H "X-Admin: $SB_ADMIN" "$URL/api/files/$f" >/dev/null && echo "reset   $f"
  else
    curl -fsS -X PUT -H "X-Admin: $SB_ADMIN" --data-binary @"$f" "$URL/api/files/$f" >/dev/null && echo "pushed  $f"
  fi
done
echo "→ reload the dashboard in the browser"
