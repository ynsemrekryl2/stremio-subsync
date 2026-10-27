#!/usr/bin/env bash
# One-time server setup on Ubuntu 24.04 (Oracle Always Free ARM or any VPS).
# deploy.sh runs it with sudo on the first deploy. Safe to re-run: the token
# is kept.
set -euo pipefail

domain="${1:?usage: setup.sh <domain> <source dir>}"
src="${2:?usage: setup.sh <domain> <source dir>}"
env_file=/etc/stremio-subsync.env

apt-get update
apt-get install -y build-essential caddy curl

# Oracle's Ubuntu images reject everything but SSH in iptables; open the web
# ports above that rule and persist them.
for port in 80 443; do
	iptables -C INPUT -p tcp --dport "$port" -m state --state NEW -j ACCEPT 2>/dev/null ||
		iptables -I INPUT -p tcp --dport "$port" -m state --state NEW -j ACCEPT
done
if command -v netfilter-persistent >/dev/null; then
	netfilter-persistent save
fi

if [ ! -f "$env_file" ]; then
	umask 077
	printf 'SUBSYNC_TOKEN=%s\nSUBSYNC_DOMAIN=%s\n' "$(openssl rand -hex 32)" "$domain" >"$env_file"
fi
chmod 600 "$env_file"

install -m 644 "$src/deploy/stremio-subsync.service" /etc/systemd/system/
sed "s/SUBSYNC_DOMAIN/$domain/" "$src/deploy/Caddyfile" >/etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile

systemctl daemon-reload
systemctl enable stremio-subsync
systemctl reload-or-restart caddy
