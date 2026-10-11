#!/usr/bin/env bash
# Local check of site/deploy (M13 Phase 0, throwaway): deploy with update.sh into a
# scratch root, run Debian's nginx with the real config (paths and port rewritten so
# it runs unprivileged), a fake Plausible on 127.0.0.1:8000, and curl it all.
set -euo pipefail
S=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$S/../.." && pwd)
NGINX=$S/nginx-root/usr/sbin/nginx
PORT=18080
rm -rf "$S/srv" "$S/etc" "$S/log" "$S/origin" "$S/run"
mkdir -p "$S/srv" "$S/etc/snippets" "$S/etc/sites" "$S/log/getfaro" "$S/run"

echo "== 1. deploy: a throwaway git origin with the current site/ files"
git init -q "$S/origin" && cp -R "$REPO/site" "$S/origin/" && git -C "$S/origin" add -A && git -C "$S/origin" -c user.email=t@t -c user.name=t commit -qm one
git clone -q "$S/origin" "$S/srv/repo"
FARO_SITE_ROOT=$S/srv "$S/srv/repo/site/deploy/update.sh"
for i in 2 3 4 5 6 7; do  # more commits: pruning keeps 5
    echo "$i" > "$S/origin/site/placeholder/n.txt"; git -C "$S/origin" add -A; git -C "$S/origin" -c user.email=t@t -c user.name=t commit -qm "$i"
    FARO_SITE_ROOT=$S/srv "$S/srv/repo/site/deploy/update.sh" >/dev/null
done
FARO_SITE_ROOT=$S/srv "$S/srv/repo/site/deploy/update.sh"   # same commit again: no rebuild
echo "releases kept: $(ls "$S/srv/releases" | wc -l); current -> $(readlink "$S/srv/current"); head: $(git -C "$S/origin" rev-parse --short=12 HEAD)"
ls "$S/srv/current" "$S/srv/current/js"

echo "== 2. nginx config"
printf 'types { text/html html; text/css css; application/javascript js; application/json json; image/svg+xml svg; application/wasm wasm; text/plain txt; }\n' > "$S/etc/mime.types"
cp "$REPO/site/deploy/nginx/snippets/getfaro-headers.conf" "$S/etc/snippets/"
for f in getfaro.dev.conf plausible.harababurel.com.conf; do
    sed -e "s#/etc/nginx/snippets#$S/etc/snippets#; s#/var/log/nginx#$S/log#; s#/srv/getfaro.dev#$S/srv#; s#listen 80;#listen $PORT;#; s#PLAUSIBLE_SCRIPT#pa-TESTSCRIPT#" \
        "$REPO/site/deploy/nginx/$f" > "$S/etc/sites/$f"
done
cat > "$S/etc/nginx.conf" <<EOF
pid $S/run/nginx.pid;
error_log $S/log/error.log;
events {}
http {
    include $S/etc/mime.types;
    access_log $S/log/access.log;
    client_body_temp_path $S/run/body; proxy_temp_path $S/run/proxy; fastcgi_temp_path $S/run/fcgi; uwsgi_temp_path $S/run/uwsgi; scgi_temp_path $S/run/scgi;
    include $S/etc/sites/*;
}
EOF
"$NGINX" -t -p "$S/run" -c "$S/etc/nginx.conf"

echo "== 3. run: fake Plausible + nginx"
python3 "$S/fake_plausible.py" & FAKE=$!
"$NGINX" -p "$S/run" -c "$S/etc/nginx.conf"
trap 'kill $FAKE 2>/dev/null; "$NGINX" -p "$S/run" -c "$S/etc/nginx.conf" -s stop 2>/dev/null || true' EXIT
sleep 1
h() { curl -s -D - -o /dev/null -H "Host: $1" "http://127.0.0.1:$PORT$2" "${@:3}"; }
echo "-- GET / (getfaro.dev)"; h getfaro.dev / | tr -d '\r'
echo "-- GET / gzip"; h getfaro.dev / -H 'Accept-Encoding: gzip' | tr -d '\r' | grep -iE '^(HTTP|content-encoding|vary)'
echo "-- GET /nope"; h getfaro.dev /nope | head -1
echo "-- www redirect"; h www.getfaro.dev /x?y=1 | tr -d '\r' | grep -iE '^(HTTP|location)'
echo "-- /js/s.js"; h getfaro.dev /js/s.js | tr -d '\r' | grep -iE '^(HTTP|cache-control|content-security)'
curl -s -H 'Host: getfaro.dev' "http://127.0.0.1:$PORT/js/s.js"; echo
echo "-- /api/event"; curl -s -X POST -H 'Host: getfaro.dev' -H 'Content-Type: application/json' -d '{"name":"pageview"}' "http://127.0.0.1:$PORT/api/event"; echo
echo "-- dashboard vhost (websocket upgrade header passes)"; curl -s -H 'Host: plausible.harababurel.com' -H 'Upgrade: websocket' -H 'Connection: Upgrade' "http://127.0.0.1:$PORT/live/websocket"; echo
echo "-- logs written to"; ls "$S/log/getfaro"

echo "== 4. write-env.sh"
mkdir -p "$S/run/plausible"; "$REPO/site/deploy/plausible/write-env.sh" "$S/run/plausible"; stat -c '%a %n' "$S/run/plausible/.env"; sed 's/=.*/=…/' "$S/run/plausible/.env"
"$REPO/site/deploy/plausible/write-env.sh" "$S/run/plausible" 2>&1 || echo "(refused to overwrite, as intended)"

echo "== 5. logrotate syntax"
sed "s#/var/log/nginx/getfaro#$S/log/getfaro#; s#create 0640 www-data adm#create 0640#" "$REPO/site/deploy/logrotate/getfaro" > "$S/run/logrotate.conf"
logrotate -d -s "$S/run/logrotate.state" "$S/run/logrotate.conf" 2>&1 | grep -iE 'error|rotating pattern|considering' | head -5
