#!/usr/bin/env bash
# Starts the local server with the install token and the AIOStreams link kept
# in the macOS Keychain, so the Stremio install link stays the same across runs.
# The first run creates the token, asks for the AIOStreams link (hidden input)
# and copies the install link to the clipboard. Neither secret is printed.
#
#   scripts/dev.sh             start the server (extra SUBSYNC_* vars pass through)
#   scripts/dev.sh link        copy the install link to the clipboard
#   scripts/dev.sh dashboard   open the live event view in the browser
#   scripts/dev.sh subdl       save a SubDL API key (hidden input); later starts use it
#   scripts/dev.sh openrouter  save an OpenRouter API key (hidden input)
#   scripts/dev.sh translate-check <english.srt> [out.srt]
#                              translate a local SRT with that key; prints counts only
#   scripts/dev.sh balance-check <english.srt> <translated.srt>
#                              count split sentences moved between cues; no key, no network
#   scripts/dev.sh forget      remove the secrets; the next start makes a new token
set -euo pipefail

cd "$(dirname "$0")/.."
service="${SUBSYNC_KEYCHAIN_SERVICE:-stremio-subsync}"
bind="${SUBSYNC_BIND:-127.0.0.1:7001}"

secret() { security find-generic-password -s "$service" -a "$1" -w 2>/dev/null || true; }
store() { security add-generic-password -U -s "$service" -a "$1" -w "$2"; }
base_url() { echo "http://$bind/$(secret token)"; }

case "${1:-start}" in
start) ;;
link)
	[ -n "$(secret token)" ] || { echo "no token yet: run scripts/dev.sh first" >&2; exit 1; }
	echo "$(base_url)/manifest.json" | pbcopy
	echo "install link copied to the clipboard"
	exit 0
	;;
dashboard)
	[ -n "$(secret token)" ] || { echo "no token yet: run scripts/dev.sh first" >&2; exit 1; }
	open "$(base_url)/dashboard"
	exit 0
	;;
subdl)
	read -rsp "SubDL API key (hidden, Enter): " key
	echo
	[ -n "$key" ] || { echo "empty key, nothing saved" >&2; exit 1; }
	store subdl-key "$key"
	unset key
	echo "SubDL key saved; restart the server to use it"
	exit 0
	;;
openrouter)
	read -rsp "OpenRouter API key (hidden, Enter): " key
	echo
	[ -n "$key" ] || { echo "empty key, nothing saved" >&2; exit 1; }
	store openrouter-key "$key"
	unset key
	echo "OpenRouter key saved"
	exit 0
	;;
translate-check)
	shift
	[ -n "$(secret openrouter-key)" ] || { echo "no OpenRouter key: run scripts/dev.sh openrouter" >&2; exit 1; }
	OPENROUTER_API_KEY="$(secret openrouter-key)" exec cargo run --release -- translate-check "$@"
	;;
balance-check)
	shift
	exec cargo run --release -- balance-check "$@"
	;;
forget)
	security delete-generic-password -s "$service" -a token >/dev/null 2>&1 || true
	security delete-generic-password -s "$service" -a aiostreams-url >/dev/null 2>&1 || true
	security delete-generic-password -s "$service" -a subdl-key >/dev/null 2>&1 || true
	security delete-generic-password -s "$service" -a openrouter-key >/dev/null 2>&1 || true
	echo "secrets removed from the Keychain ($service)"
	exit 0
	;;
*)
	echo "usage: scripts/dev.sh [start|link|dashboard|subdl|openrouter|translate-check|balance-check|forget]" >&2
	exit 2
	;;
esac

if [ -z "$(secret token)" ]; then
	store token "$(openssl rand -hex 24)"
	echo "$(base_url)/manifest.json" | pbcopy
	echo "new install token saved; install link copied to the clipboard"
	echo "paste it into the Stremio add-on search box (remove the old SubSync first)"
fi
if [ -z "$(secret aiostreams-url)" ]; then
	read -rsp "AIOStreams manifest link (hidden, Enter): " url
	echo
	[ -n "$url" ] || { echo "empty link, nothing saved" >&2; exit 1; }
	store aiostreams-url "$url"
	unset url
fi

SUBSYNC_TOKEN="$(secret token)"
SUBSYNC_AIOSTREAMS_URL="$(secret aiostreams-url)"
export SUBSYNC_TOKEN SUBSYNC_AIOSTREAMS_URL
# Optional second subtitle source; an explicit SUBDL_API_KEY wins.
if [ -z "${SUBDL_API_KEY:-}" ] && [ -n "$(secret subdl-key)" ]; then
	SUBDL_API_KEY="$(secret subdl-key)"
	export SUBDL_API_KEY
fi
exec cargo run
