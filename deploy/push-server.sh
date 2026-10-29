#!/bin/sh
# Rebuild the server and swap it into the RouterOS container on the main
# router. Data, config.json and UI overrides live in the sb-data mount and
# survive. Needs the tunnel (REST :40080, SOCKS :41080).
#   ./deploy/push-server.sh
set -e
cd "$(dirname "$0")/.."
T=~/.solarbox
C="admin:$(cat $T/.router-pw)"; B=http://127.0.0.1:40080/rest; H='content-type: application/json'
./deploy/build-armv5.sh
echo ">> upload"
curl -sS -m120 --socks5-hostname 127.0.0.1:41080 -T solarbox-armv5.tar "ftp://admin:$(cat $T/.router-pw)@192.168.88.1/" -o /dev/null
state() { curl -s -m8 -u "$C" $B/container | python3 -c 'import json,sys;c=[x for x in json.load(sys.stdin) if x.get("name")=="solarbox"][0];print("running" if c.get("running")=="true" else "stopped")'; }
echo ">> stop"; curl -sS -m10 -u "$C" -X POST $B/container/stop -H "$H" -d '{".id":"solarbox"}' -o /dev/null
i=0; while [ "$(state)" != stopped ] && [ $i -lt 20 ]; do sleep 2; i=$((i+1)); done
echo ">> repull"; curl -sS -m30 -u "$C" -X POST $B/container/repull -H "$H" -d '{".id":"solarbox"}' -o /dev/null
sleep 6
echo ">> start"; curl -sS -m10 -u "$C" -X POST $B/container/start -H "$H" -d '{".id":"solarbox"}' -o /dev/null
i=0; until curl -sf -m3 http://127.0.0.1:48080/healthz >/dev/null || [ $i -ge 20 ]; do sleep 2; i=$((i+1)); done
curl -sf -m3 http://127.0.0.1:48080/healthz >/dev/null && echo ">> up" || { echo ">> NOT answering — check /log on the router"; exit 1; }
# The image is extracted into the container's root-dir; the tar only wastes flash now.
curl -sS -m10 -u "$C" -X POST $B/file/remove -H "$H" -d '{"numbers":"solarbox-armv5.tar"}' -o /dev/null && echo ">> removed uploaded tar from router flash"
