#!/bin/sh
# Rebuild the server and swap it into the RouterOS container. Data,
# config.json and UI overrides live in the sb-data mount and survive.
# Router access comes from deploy/local.env (see local.env.example).
#   ./deploy/push-server.sh
set -e
cd "$(dirname "$0")/.."
[ -f deploy/local.env ] && . deploy/local.env
: "${SB_REST:?set SB_REST in deploy/local.env}" "${SB_FTP_HOST:?}" "${SB_ROUTER_PW_FILE:?}" "${SB_URL:?}"
PW=$(cat "$(eval echo "$SB_ROUTER_PW_FILE")")
C="${SB_ROUTER_USER:-admin}:$PW"; B=$SB_REST; H='content-type: application/json'
./deploy/build-armv5.sh
echo ">> upload"
curl -sS -m120 ${SB_SOCKS:+--socks5-hostname "$SB_SOCKS"} -T solarbox-armv5.tar "ftp://$C@$SB_FTP_HOST/" -o /dev/null
state() { curl -s -m8 -u "$C" $B/container | python3 -c 'import json,sys;c=[x for x in json.load(sys.stdin) if x.get("name")=="solarbox"][0];print("running" if c.get("running")=="true" else "stopped")'; }
echo ">> stop"; curl -sS -m10 -u "$C" -X POST $B/container/stop -H "$H" -d '{".id":"solarbox"}' -o /dev/null
i=0; while [ "$(state)" != stopped ] && [ $i -lt 20 ]; do sleep 2; i=$((i+1)); done
echo ">> repull"; curl -sS -m30 -u "$C" -X POST $B/container/repull -H "$H" -d '{".id":"solarbox"}' -o /dev/null
sleep 6
echo ">> start"; curl -sS -m10 -u "$C" -X POST $B/container/start -H "$H" -d '{".id":"solarbox"}' -o /dev/null
i=0; until curl -sf -m3 "$SB_URL/healthz" >/dev/null || [ $i -ge 20 ]; do sleep 2; i=$((i+1)); done
curl -sf -m3 "$SB_URL/healthz" >/dev/null && echo ">> up" || { echo ">> NOT answering — check /log on the router"; exit 1; }
# The image is extracted into the container's root-dir; the tar only wastes flash now.
curl -sS -m10 -u "$C" -X POST $B/file/remove -H "$H" -d '{"numbers":"solarbox-armv5.tar"}' -o /dev/null && echo ">> removed uploaded tar from router flash"
