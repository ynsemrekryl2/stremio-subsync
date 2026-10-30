# Kurulum (VPS + HTTPS)

Hedef: Oracle Cloud **Always Free** ARM sunucu (ücretsiz), Ubuntu 24.04.
Caddy HTTPS sertifikasını Let's Encrypt'ten kendisi alır ve yeniler. Uygulama
`127.0.0.1:7001`'de systemd servisi olarak çalışır; dışarıya yalnız Caddy açıktır.

## 1. Sunucuyu aç (Oracle konsolu)

- Compute → Instances → Create instance
- Image: **Canonical Ubuntu 24.04** (aarch64)
- Shape: **VM.Standard.A1.Flex**, 1 OCPU / 6 GB yeterli (ücretsiz sınır 4 OCPU / 24 GB)
- SSH: mevcut public key'ini ekle
- "Out of capacity" hatası alırsan başka bir Availability Domain'i dene ya da
  bir süre sonra tekrar dene.

## 2. Ağ

- Instance → Subnet → Security List → Ingress kuralı ekle:
  kaynak `0.0.0.0/0`, TCP, hedef port `80` ve `443`.
- Alan adında bir **A kaydı** (ör. `subs.ornek.com`) → sunucunun public IP'si.
  Cloudflare kullanıyorsan kayıt **DNS only** (gri bulut) olmalı.

## 3. Deploy (Mac'ten)

İlk sefer:

```bash
SUBSYNC_HOST=ubuntu@<ip> SUBSYNC_DOMAIN=<alan adı> deploy/deploy.sh
```

Sonraki seferler yalnız `SUBSYNC_HOST` ister. İlk derleme birkaç dakika sürer.
Token sunucuda üretilir ve `/etc/stremio-subsync.env` dosyasında (yalnız root
okuyabilir) durur.

## 4. Kurulum linki

Token'ı ekrana basmadan panoya kopyalar:

```bash
ssh ubuntu@<ip> 'sudo sh -c ". /etc/stremio-subsync.env; echo https://\$SUBSYNC_DOMAIN/\$SUBSYNC_TOKEN/manifest.json"' | pbcopy
```

## Loglar

- Uygulama: `journalctl -u stremio-subsync` — token, dosya adı vb. basılmaz.
- Caddy: erişim logu kapalı; URI basan hata logları `deploy/Caddyfile`'da
  devre dışı. Yeni bir `log` direktifi eklenirse token loglara düşer.

## Bilinen risk

Oracle, Always Free hesaplarda 7 gün boyunca boşta kalan sunucuları geri
alabiliyor. Bu add-on çok az CPU kullanacağı için risk var. Hesabı
Pay-As-You-Go'ya yükseltmek bunu kaldırır (ücretsiz sınırlar içinde kalındıkça
ücret çıkmaz, ama kart gerekir).
