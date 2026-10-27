#!/usr/bin/env bash
# Copies the source to the server, builds there and restarts the service.
# Run from the repo root. The first run also sets the server up:
#   SUBSYNC_HOST=ubuntu@203.0.113.10 SUBSYNC_DOMAIN=subs.example.com deploy/deploy.sh
# Later runs need only SUBSYNC_HOST. Extra ssh options go in ~/.ssh/config.
set -euo pipefail

host="${SUBSYNC_HOST:?set SUBSYNC_HOST, e.g. ubuntu@203.0.113.10}"
domain="${SUBSYNC_DOMAIN:-}"
src=stremio-subsync-src

rsync -az --delete --exclude /target --exclude /.git ./ "$host:$src/"
ssh "$host" bash -s <<EOF
set -euo pipefail
if [ ! -f /etc/stremio-subsync.env ]; then
	[ -n "$domain" ] || { echo "first deploy: set SUBSYNC_DOMAIN" >&2; exit 2; }
	sudo bash \$HOME/$src/deploy/setup.sh "$domain" \$HOME/$src
fi
if [ ! -x \$HOME/.cargo/bin/cargo ]; then
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
		sh -s -- -y --profile minimal --default-toolchain none
fi
cd \$HOME/$src
\$HOME/.cargo/bin/cargo build --release --locked
sudo install -m 755 target/release/stremio-subsync /usr/local/bin/
sudo systemctl restart stremio-subsync
systemctl is-active stremio-subsync
EOF
