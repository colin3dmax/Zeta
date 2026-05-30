#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
HOST="${ZETA_DEPLOY_HOST:-8.153.206.212}"
USER_HOST="${ZETA_DEPLOY_USER_HOST:-root@$HOST}"
WEB_ROOT="${ZETA_WEB_ROOT:-/var/www/zeta.jennieapp.com}"
NGINX_AVAILABLE="${ZETA_NGINX_AVAILABLE:-/etc/nginx/sites-available/zeta.jennieapp.com.conf}"
NGINX_ENABLED="${ZETA_NGINX_ENABLED:-/etc/nginx/sites-enabled/zeta.jennieapp.com.conf}"
SSH_OPTS="-o ProxyCommand=none -o ProxyJump=none"

unset http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY

cd "$ROOT"
sh tools/build-website.sh

ssh $SSH_OPTS "$USER_HOST" "mkdir -p '$WEB_ROOT'"
rsync -avz --delete -e "ssh $SSH_OPTS" "$ROOT/website/dist/" "$USER_HOST:$WEB_ROOT/"
rsync -avz -e "ssh $SSH_OPTS" "$ROOT/website/deploy/nginx-zeta.jennieapp.com.conf" "$USER_HOST:/tmp/zeta.jennieapp.com.conf"

ssh $SSH_OPTS "$USER_HOST" \
  "cp /tmp/zeta.jennieapp.com.conf '$NGINX_AVAILABLE' && \
   ln -sfn '$NGINX_AVAILABLE' '$NGINX_ENABLED' && \
   nginx -t && \
   systemctl reload nginx"

curl --noproxy '*' -fsSI https://zeta.jennieapp.com/ >/dev/null
curl --noproxy '*' -fsSI https://zeta.jennieapp.com/zeta.wasm >/dev/null

echo "deployed https://zeta.jennieapp.com/"
