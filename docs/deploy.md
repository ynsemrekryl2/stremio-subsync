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
  OCI CLI ile (oturum yoksa önce `oci session authenticate --region eu-turin-1`):
  `oci network security-list update --ingress-security-rules file://...`
  mevcut kuralların **tamamını** değiştirir; önce `get` ile alıp 80/443'ü
  ekleyerek gönder. Kural yoksa Let's Encrypt sunucuya ulaşamaz ve Caddy
  sertifika alamaz (log'da `challenge failed ... connection`).
- Ad: kendi alan adı gerekmez. Şu an `92-4-220-81.sslip.io` kullanılıyor
  (IP'yi tire ile yazan ücretsiz isim). Kendi alan adına geçersen bir **A kaydı**
  (ör. `subs.ornek.com`) → sunucunun public IP'si; Cloudflare kullanıyorsan
  kayıt **DNS only** (gri bulut) olmalı.

## 3. Deploy (Mac'ten)

Ön koşul: `scripts/dev.sh` Mac'te en az bir kez çalışmış olmalı. AIOStreams
linki (ve varsa SubDL, AltyaziDB, SubSource ve Wyzie anahtarları) oradaki Anahtar Zinciri girdilerinden
alınır, ssh'in standart girdisiyle sunucuya gider; ekrana ve komut satırına
düşmez.

İlk sefer:

```bash
SUBSYNC_HOST=oracle SUBSYNC_DOMAIN=92-4-220-81.sslip.io deploy/deploy.sh
```

Sonraki seferler yalnız `SUBSYNC_HOST` ister. İlk derleme 1 OCPU'da uzun
sürer (Rust araç zinciri de kurulur). Her deploy:

1. Kaynağı `rsync` ile `~/stremio-subsync-src/`'ye kopyalar; `.gitignore`'a
   uyduğu için `cache/`, `dumps/`, `.local/`, worktree'ler ve `target/` gitmez.
2. İlk seferde `deploy/setup.sh` çalışır: Caddy, güvenlik duvarı, systemd
   birimi ve yeni bir token.
3. AIOStreams linkini, SubDL, AltyaziDB, SubSource ve Wyzie anahtarlarını `deploy/env.sh` ile env dosyasına
   yazar (Anahtar Zinciri'nde olmayan anahtar sunucudan da silinir).
4. Sunucuda derler, servisi yeniden başlatır.

Yalnız gizli değerleri güncellemek (örneğin AIOStreams şifresi
değişince önce `scripts/dev.sh forget`, sonra `scripts/dev.sh` ile yeniden
kaydedip):

```bash
SUBSYNC_HOST=ubuntu@<ip> deploy/deploy.sh secrets
```

Sunucudaki dosyalar:

| Yol | İçerik |
|---|---|
| `/etc/stremio-subsync.env` | Token, alan adı, AIOStreams linki, SubDL, AltyaziDB, SubSource ve Wyzie anahtarları (yalnız root, 0600) |
| `/etc/systemd/system/stremio-subsync.service` | Servis birimi (`deploy/stremio-subsync.service`) |
| `/etc/caddy/Caddyfile` | HTTPS reverse proxy (`deploy/Caddyfile`) |
| `/var/lib/stremio-subsync/cache/` | Sunulan menülerin cache'i; deploy ve yeniden başlatmada korunur |
| `/usr/local/bin/stremio-subsync` | Derlenmiş uygulama |

Dump kapalıdır (`SUBSYNC_DUMP_DIR` verilmez); ölçümler yerelde yapılır.

## 4. Kurulum linki

Sunucunun token'ı yerelin ya da Funnel'ın token'ından farklıdır. Linki ekrana
basmadan panoya kopyalar:

```bash
SUBSYNC_HOST=ubuntu@<ip> deploy/deploy.sh link
```

Stremio'da eski SubSync kurulumunu kaldırıp bu linki kur. Olay paneli aynı
token'la `https://<alan adı>/<token>/dashboard` adresinde.

## Loglar

- Uygulama: `journalctl -u stremio-subsync` — token, dosya adı vb. basılmaz.
- Caddy: erişim logu kapalı; URI basan hata logları `deploy/Caddyfile`'da
  devre dışı. Yeni bir `log` direktifi eklenirse token loglara düşer.

## Bilinen risk

Oracle, Always Free hesaplarda 7 gün boyunca boşta kalan sunucuları geri
alabiliyor. Bu add-on çok az CPU kullanacağı için risk var. Hesabı
Pay-As-You-Go'ya yükseltmek bunu kaldırır (ücretsiz sınırlar içinde kalındıkça
ücret çıkmaz, ama kart gerekir).
