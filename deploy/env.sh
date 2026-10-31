#!/usr/bin/env bash
# Stores the service's secrets in /etc/stremio-subsync.env from KEY=VALUE
# lines on stdin. deploy.sh pipes them over ssh from the Mac's Keychain, so
# they never reach a command line or a terminal. Only known keys are taken;
# an empty value removes the key. Values are never printed.
set -euo pipefail
umask 077

env_file=/etc/stremio-subsync.env
allowed=" SUBSYNC_AIOSTREAMS_URL SUBDL_API_KEY ALTYAZIDB_API_KEY SUBSOURCE_API_KEY OPENROUTER_API_KEY "
[ -f "$env_file" ] || { echo "$env_file missing: run setup.sh first" >&2; exit 1; }

tmp="$(mktemp "$env_file.XXXXXX")"
trap 'rm -f "$tmp" "$tmp.next"' EXIT
cp "$env_file" "$tmp"
while IFS= read -r line; do
	key="${line%%=*}"
	value="${line#*=}"
	case "$allowed" in
	*" $key "*) ;;
	*) echo "skipped a line without a known key" >&2; continue ;;
	esac
	# systemd reads unquoted values; these would change their meaning.
	case "$value" in
	*[[:space:]\\\"\']*) echo "$key refused: whitespace, quote or backslash" >&2; exit 2 ;;
	esac
	grep -v "^$key=" "$tmp" >"$tmp.next" || true
	mv "$tmp.next" "$tmp"
	if [ -n "$value" ]; then
		printf '%s=%s\n' "$key" "$value" >>"$tmp"
		echo "$key: set"
	else
		echo "$key: removed"
	fi
done
chmod 600 "$tmp"
mv "$tmp" "$env_file"
