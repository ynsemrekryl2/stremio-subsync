#!/usr/bin/env bash
# Installs or updates the SubSync server on Debian or Ubuntu (SS-066):
#
#   curl -fsSL https://raw.githubusercontent.com/ynsemrekryl2/stremio-subsync/main/install.sh | sudo bash
#
# Downloads the latest release for this machine, sets up Caddy (HTTPS from
# Let's Encrypt), the firewall and the systemd service, then prints the
# configure page's link. Running it again updates the program and keeps the
# token, the address, the keys and the cache. Options go after `sudo`:
#
#   SUBSYNC_DOMAIN=subs.example.com   own domain instead of <ip>.sslip.io
#   SUBSYNC_VERSION=v0.2.0            a given release instead of the latest
#   SUBSYNC_REF=main                  branch the unit, Caddyfile and `subsync` come from
#
# The link is printed once, here, for the person installing; it never goes
# to a log. `subsync link` shows it again.
set -euo pipefail

repo=ynsemrekryl2/stremio-subsync
ref="${SUBSYNC_REF:-main}"
# SUBSYNC_RAW=file:///src takes those files from a local checkout (testing).
raw="${SUBSYNC_RAW:-https://raw.githubusercontent.com/$repo/$ref}"
releases="https://github.com/$repo/releases"
env_file=/etc/stremio-subsync.env
secrets_file=/var/lib/stremio-subsync/secrets.json
caddyfile=/etc/caddy/Caddyfile
marker="# Written by stremio-subsync"

step() { printf '\n\033[1m%s\033[0m\n' "$*"; }
fail() {
	printf '\nHata: %s\n' "$*" >&2
	exit 1
}

[ "$(id -u)" = 0 ] || fail "root olarak çalıştır: curl … | sudo bash"
command -v apt-get >/dev/null && command -v systemctl >/dev/null ||
	fail "yalnız Debian ve Ubuntu (apt + systemd) destekleniyor"
case "$(uname -m)" in
x86_64 | aarch64) arch="$(uname -m)" ;;
*) fail "desteklenmeyen işlemci: $(uname -m) (x86_64 ya da aarch64 gerekir)" ;;
esac
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

step "1/6 Gerekli paketler"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq curl ca-certificates openssl >/dev/null
if ! command -v caddy >/dev/null; then
	candidate="$(apt-cache policy caddy 2>/dev/null | awk '/Candidate:/ { print $2 }')"
	if [ -z "$candidate" ] || [ "$candidate" = "(none)" ]; then
		# Older releases have no caddy package; use Caddy's own repository.
		apt-get install -y -qq gnupg >/dev/null
		curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key |
			gpg --dearmor --yes -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
		curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt \
			>/etc/apt/sources.list.d/caddy-stable.list
		apt-get update -qq
	fi
	apt-get install -y -qq caddy >/dev/null
fi
echo "curl, openssl, caddy hazır"

step "2/6 Program"
if [ -n "${SUBSYNC_VERSION:-}" ]; then
	from="$releases/download/$SUBSYNC_VERSION"
else
	from="$releases/latest/download"
fi
asset="stremio-subsync-$arch-linux"
curl -fsSL -o "$tmp/$asset" "$from/$asset" || fail "program indirilemedi ($from/$asset)"
curl -fsSL -o "$tmp/$asset.sha256" "$from/$asset.sha256" || fail "sağlama dosyası indirilemedi"
(cd "$tmp" && sha256sum -c --quiet "$asset.sha256") || fail "indirilen program bozuk: sağlama tutmadı"
chmod 755 "$tmp/$asset"
new="$("$tmp/$asset" --version)" || fail "program bu makinede çalışmadı (glibc 2.35 ya da yenisi gerekir)"
if [ -x /usr/local/bin/stremio-subsync ]; then
	# Builds before 0.2.0 have no --version.
	old="$(/usr/local/bin/stremio-subsync --version 2>/dev/null | grep '^stremio-subsync ' || echo "önceki sürüm")"
else
	old="yok"
fi
for file in deploy/stremio-subsync.service deploy/Caddyfile deploy/subsync; do
	curl -fsSL -o "$tmp/${file#deploy/}" "$raw/$file" || fail "$file indirilemedi"
done
install -m 755 "$tmp/$asset" /usr/local/bin/stremio-subsync
install -m 755 "$tmp/subsync" /usr/local/bin/subsync
echo "${old#stremio-subsync } → ${new#stremio-subsync }"

step "3/6 Adres ve kurulum token'ı"
setting() { [ -f "$env_file" ] && awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); print }' "$env_file"; }
domain="${SUBSYNC_DOMAIN:-$(setting SUBSYNC_DOMAIN || true)}"
if [ -z "$domain" ]; then
	ip="$(curl -fsS4 --max-time 10 https://api.ipify.org || curl -fsS4 --max-time 10 https://ifconfig.me)" ||
		fail "sunucunun IP'si bulunamadı; alan adını SUBSYNC_DOMAIN=… ile ver"
	domain="${ip//./-}.sslip.io"
fi
case "$domain" in
*[!a-z0-9.-]* | "" | .* | *.) fail "geçersiz alan adı: $domain" ;;
esac
umask 077
if [ ! -f "$env_file" ]; then
	# The install link's secret: the only thing between the internet and the keys.
	printf 'SUBSYNC_TOKEN=%s\nSUBSYNC_DOMAIN=%s\n' "$(openssl rand -hex 32)" "$domain" >"$env_file"
	echo "yeni token üretildi"
elif [ "$(setting SUBSYNC_DOMAIN || true)" != "$domain" ]; then
	grep -v '^SUBSYNC_DOMAIN=' "$env_file" >"$tmp/env" || true
	printf 'SUBSYNC_DOMAIN=%s\n' "$domain" >>"$tmp/env"
	cat "$tmp/env" >"$env_file"
	echo "mevcut token korundu, adres değişti"
else
	echo "mevcut token ve adres korundu"
fi
chmod 600 "$env_file"
umask 022
echo "adres: $domain"

step "4/6 Güvenlik duvarı (80 ve 443)"
if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q "Status: active"; then
	ufw allow 80/tcp >/dev/null
	ufw allow 443/tcp >/dev/null
	echo "ufw: 80 ve 443 açıldı"
elif command -v iptables >/dev/null; then
	# Oracle's Ubuntu images reject everything but SSH in iptables.
	for port in 80 443; do
		iptables -C INPUT -p tcp --dport "$port" -m state --state NEW -j ACCEPT 2>/dev/null ||
			iptables -I INPUT -p tcp --dport "$port" -m state --state NEW -j ACCEPT
	done
	if command -v netfilter-persistent >/dev/null; then
		netfilter-persistent save >/dev/null 2>&1 || true
	fi
	echo "iptables: 80 ve 443 açık"
else
	echo "yerel güvenlik duvarı yok"
fi

step "5/6 Servis ve HTTPS"
install -m 644 "$tmp/stremio-subsync.service" /etc/systemd/system/
# Ours (marked, or from an install before the mark) or the package's
# default page is replaced as it is.
if [ -f "$caddyfile" ] && ! grep -qF "$marker" "$caddyfile" && grep -qv '^\s*\(#.*\)\?$' "$caddyfile" &&
	! grep -q '/usr/share/caddy' "$caddyfile" && ! grep -q 'reverse_proxy 127.0.0.1:7001' "$caddyfile"; then
	# Someone else's site config: keep a copy rather than lose it.
	cp "$caddyfile" "$caddyfile.before-subsync"
	echo "önceki Caddyfile $caddyfile.before-subsync olarak saklandı"
fi
sed "s/SUBSYNC_DOMAIN/$domain/" "$tmp/Caddyfile" >"$tmp/Caddyfile.site"
caddy validate --config "$tmp/Caddyfile.site" --adapter caddyfile >/dev/null 2>&1 || fail "Caddy ayarı geçersiz"
install -m 644 "$tmp/Caddyfile.site" "$caddyfile"
systemctl daemon-reload
systemctl enable --quiet stremio-subsync caddy
systemctl restart stremio-subsync
systemctl reload-or-restart caddy
echo "stremio-subsync ve caddy çalışıyor"

step "6/6 Kontrol"
token() {
	local t
	t="$(sed -n 's/^ *"token": *"\([A-Za-z0-9_-]*\)".*/\1/p' "$secrets_file" 2>/dev/null || true)"
	[ -n "$t" ] || t="$(setting SUBSYNC_TOKEN)"
	printf '%s' "$t"
}
local_ok=""
for _ in $(seq 30); do
	if curl -fs -o /dev/null --max-time 2 "http://127.0.0.1:7001/$(token)/manifest.json"; then
		local_ok=1
		break
	fi
	sleep 1
done
[ -n "$local_ok" ] || fail "eklenti açılmadı; nedeni: journalctl -u stremio-subsync -n 30"
echo "eklenti cevap veriyor"
public_ok=""
echo "dışarıdan HTTPS deneniyor (sertifika ilk seferde ~30 sn sürebilir, en çok 2 dk)…"
for _ in $(seq 15); do
	if curl -fs -o /dev/null --max-time 5 "https://$domain/$(token)/manifest.json"; then
		public_ok=1
		break
	fi
	sleep 3
done
if [ -n "$public_ok" ]; then
	echo "https://$domain dışarıdan geçerli sertifikayla açılıyor"
else
	cat <<EOF
UYARI: https://$domain dışarıdan açılmadı ya da sertifika alınamadı.
En sık neden: bulut sağlayıcının güvenlik duvarı 80 ve 443'ü kapalı tutuyor.
  Oracle: Networking → Virtual cloud networks → (ağın) → Security Lists →
          Default Security List → Add Ingress Rules: kaynak 0.0.0.0/0,
          TCP, hedef port 80; aynısını 443 için.
  Diğerleri (Hetzner, AWS, …): sunucunun firewall / security group ayarında
          80 ve 443'e TCP izni ver.
Sonra: sudo systemctl restart caddy  ·  sertifika durumu: journalctl -u caddy -n 30
EOF
fi

cat <<EOF

$(printf '\033[1m')SubSync kuruldu (${new#stremio-subsync }).$(printf '\033[0m')

Configure sayfası: AIOStreams linkini ve anahtarları buradan gir,
Stremio'ya da buradan kur:

  https://$domain/$(token)/configure

Bu link kurulumun şifresidir; kimseyle paylaşma. Yeniden görmek için: subsync link
Sunucuyu yönetmek için: subsync  (durum, update, logs, backup, restore, uninstall)
EOF
