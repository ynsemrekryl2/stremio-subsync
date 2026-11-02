# Geliştirme

Normal kurulum ve kullanım README'de. Bu sayfa kodu değiştirenler için:
Mac'te yerel sunucu, kaynaktan sunucuya deploy ve testler. Ortam
değişkenleri: [`config.md`](config.md).

## Mac'te çalıştırma

Yerelde (macOS) tek komut; Finder'da `scripts/SubSync.command`'a çift
tıklamak da aynısını yapar:

```bash
scripts/dev.sh
```

İlk çalıştırmada token üretilir, AIOStreams linki (gizli girişle) sorulur,
ikisi macOS Anahtar Zinciri'ne kaydedilir ve kurulum linki panoya kopyalanır.
Sonraki açılışlar hiçbir şey sormaz; kurulum linki aynı kalır, eklentiyi
yeniden kurmak gerekmez. `scripts/dev.sh link` linki yeniden panoya kopyalar,
`scripts/dev.sh dashboard` olay panelini açar, `scripts/dev.sh subdl` SubDL
anahtarını, `scripts/dev.sh altyazidb` AltyaziDB anahtarını,
`scripts/dev.sh subsource` SubSource anahtarını, `scripts/dev.sh wyzie` Wyzie
anahtarını gizli girişle
Anahtar Zinciri'ne kaydeder (sonraki açılışlar onu kullanır), `scripts/dev.sh forget` sırları siler (sonraki açılışta yeni token). Ek `SUBSYNC_*` değişkenleri
geçer, örneğin `SUBSYNC_DUMP_DIR=dumps scripts/dev.sh`.

Elle çalıştırmak için:

```bash
export SUBSYNC_TOKEN="$(openssl rand -hex 24)"   # en az 32 karakter
cargo run
```

Stremio'ya kurulacak link ve configure sayfası:

```
http://127.0.0.1:7001/<SUBSYNC_TOKEN>/manifest.json
http://127.0.0.1:7001/<SUBSYNC_TOKEN>/configure
```

## Kaynaktan deploy (Mac'ten)

Sunucunun asıl kurulum ve güncelleme yolu `install.sh` ve `subsync update`
(GitHub sürümü, derleme yok). Henüz sürümü çıkmamış bir değişikliği denemek
için Mac'teki kaynağı sunucuda derlemek de mümkün.

Ön koşul: `scripts/dev.sh` Mac'te en az bir kez çalışmış olmalı. AIOStreams
linki (ve varsa SubDL, AltyaziDB, SubSource ve Wyzie anahtarları) oradaki Anahtar Zinciri girdilerinden
alınır, ssh'in standart girdisiyle sunucuya gider; ekrana ve komut satırına
düşmez.

Sunucu `install.sh` ile kurulduysa yalnız `SUBSYNC_HOST` yeter; boş bir
sunucuda ilk sefer `SUBSYNC_DOMAIN` de ister:

```bash
SUBSYNC_HOST=oracle deploy/deploy.sh
```

Canlı sunucuyu GitHub sürümüne geri almak için sunucuda `subsync update`.
İlk derleme 1 OCPU'da uzun sürer (Rust araç zinciri de kurulur). Her deploy:

1. Kaynağı `rsync` ile `~/stremio-subsync-src/`'ye kopyalar; `.gitignore`'a
   uyduğu için `cache/`, `dumps/`, `.local/`, worktree'ler ve `target/` gitmez.
2. İlk seferde `deploy/setup.sh` çalışır: Caddy, güvenlik duvarı, systemd
   birimi ve yeni bir token.
3. AIOStreams linkini, SubDL, AltyaziDB, SubSource ve Wyzie anahtarlarını `deploy/env.sh` ile env dosyasına
   yazar (Anahtar Zinciri'nde olmayan anahtar sunucudan da silinir).
4. Sunucuda derler, servisi yeniden başlatır.

Configure sayfasında kaydedilen bir değer env dosyasındakinin önüne geçer;
Anahtar Zinciri'nden gönderilen anahtar, sayfada kayıtlı olanı değiştirmez.

Yalnız gizli değerleri güncellemek (örneğin AIOStreams şifresi
değişince önce `scripts/dev.sh forget`, sonra `scripts/dev.sh` ile yeniden
kaydedip):

```bash
SUBSYNC_HOST=ubuntu@<ip> deploy/deploy.sh secrets
```

### Kurulum linki

Sunucunun token'ı yerelinkinden farklıdır. Linki ekrana basmadan Mac'in
panosuna kopyalar (sunucuda `subsync link` aynısını gösterir):

```bash
SUBSYNC_HOST=ubuntu@<ip> deploy/deploy.sh link
```

Panel ve configure sayfası aynı token'la `https://<alan adı>/<token>/configure`
adresinde.

## Sürüm çıkarmak

`Cargo.toml`'daki sürümü yükselt, commit et, aynı sürümle etiketle ve gönder:

```bash
git tag -a v0.3.0 -m "v0.3.0" && git push origin main v0.3.0
```

GitHub Actions (`.github/workflows/release.yml`) Intel ve ARM Linux için
derleyip sürümü yayınlar; sunucular `subsync update` ile alır. Etiket
workflow dosyasının kendisiyle aynı push'ta giderse tetiklenmeyebilir; o
zaman Actions → release → Run workflow ile etiketi elle derle.

## Test

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Testler gerçek sağlayıcı kotası, debrid trafiği veya ağ kullanmaz; her şey
sahte (fake) karşılıklarla denenir. Her push'ta GitHub Actions da çalıştırır.
