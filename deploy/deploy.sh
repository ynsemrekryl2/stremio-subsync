#!/usr/bin/env bash
# Copies the source to the server, builds there and restarts the service.
# Run from the repo root on the Mac. The first run also sets the server up:
#   SUBSYNC_HOST=ubuntu@203.0.113.10 SUBSYNC_DOMAIN=subs.example.com deploy/deploy.sh
# Later runs need only SUBSYNC_HOST. Extra ssh options go in ~/.ssh/config.
#
#   deploy/deploy.sh          sync, set up if needed, send secrets, build, restart
#   deploy/deploy.sh secrets  send secrets only, then restart
#   deploy/deploy.sh link     copy the server's install link to the clipboard
#
# The AIOStreams link, the SubDL key and the OpenRouter key come from the Keychain entries
# scripts/dev.sh keeps and go over ssh stdin; neither is printed. The
# install token is the server's own (setup.sh makes it).
set -euo pipefail

cd "$(dirname "$0")/.."
host="${SUBSYNC_HOST:?set SUBSYNC_HOST, e.g. ubuntu@203.0.113.10}"
domain="${SUBSYNC_DOMAIN:-}"
service="${SUBSYNC_KEYCHAIN_SERVICE:-stremio-subsync}"
src=stremio-subsync-src

secret() { security find-generic-password -s "$service" -a "$1" -w 2>/dev/null || true; }

send_secrets() {
	local url key
	url="$(secret aiostreams-url)"
	[ -n "$url" ] || { echo "no AIOStreams link in the Keychain: run scripts/dev.sh once" >&2; exit 1; }
	# An empty key removes it on the server: the Keychain is the source.
	key="$(secret subdl-key)"
	translator="$(secret openrouter-key)"
	printf 'SUBSYNC_AIOSTREAMS_URL=%s\nSUBDL_API_KEY=%s\nOPENROUTER_API_KEY=%s\n' "$url" "$key" "$translator" |
		ssh "$host" "sudo bash $src/deploy/env.sh"
}

case "${1:-deploy}" in
deploy) ;;
secrets)
	send_secrets
	ssh "$host" "sudo systemctl restart stremio-subsync && systemctl is-active stremio-subsync"
	exit 0
	;;
link)
	ssh "$host" 'sudo awk -F= '\''$1 == "SUBSYNC_DOMAIN" { d = $2 } $1 == "SUBSYNC_TOKEN" { t = $2 }
		END { print "https://" d "/" t "/manifest.json" }'\'' /etc/stremio-subsync.env' | pbcopy
	echo "install link copied to the clipboard"
	exit 0
	;;
*)
	echo "usage: deploy/deploy.sh [deploy|secrets|link]" >&2
	exit 2
	;;
esac

# .gitignore keeps cache, dumps, worktrees and build output on the Mac.
rsync -az --delete --exclude /.git --filter=':- .gitignore' ./ "$host:$src/"
ssh "$host" bash -s <<EOF
set -euo pipefail
if [ ! -f /etc/stremio-subsync.env ]; then
	[ -n "$domain" ] || { echo "first deploy: set SUBSYNC_DOMAIN" >&2; exit 2; }
	sudo bash \$HOME/$src/deploy/setup.sh "$domain" \$HOME/$src
fi
EOF
send_secrets
ssh "$host" bash -s <<EOF
set -euo pipefail
if [ ! -x \$HOME/.cargo/bin/cargo ]; then
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
		sh -s -- -y --profile minimal --default-toolchain none
fi
cd \$HOME/$src
\$HOME/.cargo/bin/cargo build --release --locked
sudo install -m 755 target/release/stremio-subsync /usr/local/bin/
# The unit changes too (SS-048 added a variable); setup.sh runs only once.
sudo install -m 644 deploy/stremio-subsync.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl restart stremio-subsync
systemctl is-active stremio-subsync
EOF
