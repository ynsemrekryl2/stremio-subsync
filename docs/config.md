# Ayarlar

Normal kurulumda burada bir şey değiştirmek gerekmez: AIOStreams linki,
anahtarlar ve çeviri ayarı eklentinin configure sayfasından girilir. Aşağıdaki
ortam değişkenleri yerel geliştirme ([`development.md`](development.md)) ve
özel durumlar içindir. Sunucuda `/etc/stremio-subsync.env` dosyasına
`AD=değer` satırı olarak eklenir, ardından `subsync restart`.

Configure sayfasında kaydedilen bir değer, aynı anlamdaki ortam değişkeninin
önüne geçer.

| Değişken | Varsayılan | Açıklama |
|---|---|---|
| `SUBSYNC_TOKEN` | — (zorunlu) | Kurulum linkindeki gizli parça. Linki bilen sunucuyu kullanır. |
| `SUBSYNC_BIND` | `127.0.0.1:7001` | Dinlenen adres. VPS'te HTTPS reverse proxy'nin arkasında kalır. 7000 kullanılmaz: macOS'ta AirPlay Alıcısı tutar. |
| `SUBSYNC_LANG` | `tur` | Aranan altyazı dili (ISO 639-2, OpenSubtitles adlarıyla: `tur`, `eng`, `pob`). Bu dilde uyan altyazı yoksa aynı kaynaklarda İngilizce aranır; uyan varsa menüde nedenin yanında `eng` girdisi olarak sunulur (`eng` iken kapalı). |
| `SUBSYNC_AIOSTREAMS_URL` | — | AIOStreams manifest linki; oynatılan dosyayı bulmak için. Yoksa cetvel çıkmaz, altyazı sunulmaz. Loglanmaz. |
| `SUBSYNC_RANGE_PROBE` | açık | `0` ise dosyadan cetvel okunmaz (altyazı da sunulmaz). |
| `SUBSYNC_SOURCES_PROBE` | açık | `0` ise aday altyazı aranmaz. Açıkken en iyi 3 aday indirilir. |
| `SUBSYNC_BLOCK_PROBE` | kapalı | `1` ise cetvel izinin bloklarını arka planda ölçer (SS-028); metin saklanmaz. Gerçek okuma maliyeti henüz doğrulanmadı. |
| `SUBSYNC_CACHE_DIR` | `cache` | Sunulan altyazılar burada saklanır (kabul 30 gün, "uygun yok" 1 gün). Dosya adı istek özetidir; kimlik veya dosya adı içermez. `off` ise yalnız bellek. |
| `SUBDL_API_KEY` | — | SubDL'in ücretsiz anahtarı; yoksa yalnız OpenSubtitles kullanılır. Loglanmaz. `scripts/dev.sh` bunu Anahtar Zinciri'nden alır (`scripts/dev.sh subdl`). |
| `ALTYAZIDB_API_KEY` | — | AltyaziDB'nin ücretsiz anahtarı (üye olunca); yoksa AltyaziDB aranmaz. Yalnız `tr` ve `en`. Loglanmaz. `scripts/dev.sh altyazidb` ile Anahtar Zinciri'ne; `deploy/deploy.sh` sunucuya taşır. |
| `SUBSOURCE_API_KEY` | — | SubSource'un ücretsiz anahtarı (hesap panelinin API sekmesinden); yoksa SubSource aranmaz. Arama başına 2 istek (başlık + liste), ücretsiz kota günde 7 200. Loglanmaz. `scripts/dev.sh subsource` ile Anahtar Zinciri'ne; `deploy/deploy.sh` sunucuya taşır. |
| `WYZIE_API_KEY` | — | Wyzie Subs'ın ücretsiz anahtarı (store.wyzie.io/redeem, e-posta doğrulamalı); yoksa Wyzie aranmaz. Ücretsiz anahtarla charlie ve lima kaynakları; yapay zekâ çevirileri aday sayılmaz. Ücretsiz kota günde 1 000 istek (arama ve her indirme 1). Loglanmaz. `scripts/dev.sh wyzie` ile Anahtar Zinciri'ne; `deploy/deploy.sh` sunucuya taşır. |
| `OPENROUTER_API_KEY` | — | Hedef dilde uyan altyazı yoksa uyan İngilizce altyazı bu anahtarla çevrilir ve menüde "Türkçe çeviri" girdisi olur (SS-026). Yoksa çeviri kapalı. Loglanmaz. `scripts/dev.sh openrouter` ile Anahtar Zinciri'ne; `deploy/deploy.sh` sunucuya taşır. |
| `SUBSYNC_TRANSLATE_MODEL` | `openai/gpt-6-luna` | Çeviri modeli (OpenRouter kimliği). Panelden seçilen model bunun önüne geçer (SS-048). |
| `SUBSYNC_TRANSLATE_REASONING` | `low` | Çeviri modelinin düşünme düzeyi (`none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`; hangisini kabul ettiği modele bağlı). `default` ise alan gönderilmez, modelin kendi varsayılanı geçer (GPT-6 Luna'da `medium`; düşünmeyen modeller için). `low`, Luna'da `medium`'dan ~1,7 kat hızlı ve ~%25 ucuz, kalite farkı görülmedi (SS-046). Başka düzey başka çeviri sayılır (cache ayrı). |
| `SUBSYNC_SETTINGS_FILE` | `settings.json` | Panelde seçilen çeviri modeli ve düşünme düzeyi burada saklanır; yeniden başlatma ve deploy sonrası kalır, ortam değişkenlerinin önüne geçer. Anahtar içermez. Dosya silinirse yeniden ortam değişkenleri geçer (SS-048). |
| `SUBSYNC_SECRETS_FILE` | `secrets.json` | Configure sayfasında (`/<token>/configure`, Stremio'da eklentinin Configure düğmesi) girilen AIOStreams linki ve anahtarlar burada saklanır (yalnız sahibi okur). Sayfada kaydedilen ya da kaldırılan, yukarıdaki ortam değişkenlerinin önüne geçer; kaydedince eklenti kendini yeniden başlatır. Değerler sayfaya geri gönderilmez ve loglanmaz (SS-061). |
| `SUBSYNC_DUMP_DIR` | — | Verilirse hizalanan her isteğin zamanları (yalnız sayılar) buraya JSON olarak yazılır; `cargo run -- calibrate <dizin>` bunları yeniden hizalar. |
| `SUBSYNC_LABEL_MODE` | kapalı | `1` ise indirilen her aday Stremio'nun altyazı menüsünde "SubSync 2 ✓ +1.60s · 7530" gibi görünür: sıra, kabul/ret, kaydırma, dump eki. Kaydırması olan her adayın bir de `yerel` sürümü gelir (SS-021: kayma filmin içinde değişiyorsa satırları çevresine göre ayrıca kaydırır). Etiketleme için; istek 15 sn'ye kadar bekleyebilir. |

## Sunucudaki dosyalar

| Yol | İçerik |
|---|---|
| `/etc/stremio-subsync.env` | Token ve alan adı; `deploy/deploy.sh` kullanıldıysa ortamdan gelen anahtarlar da (yalnız root, 0600) |
| `/etc/systemd/system/stremio-subsync.service` | Servis birimi (`deploy/stremio-subsync.service`) |
| `/etc/caddy/Caddyfile` | HTTPS reverse proxy (`deploy/Caddyfile`) |
| `/var/lib/stremio-subsync/secrets.json` | Configure sayfasında girilen bağlantı ve anahtarlar; yedekten yüklendiyse kurulum token'ı da. Env dosyasının önüne geçer (yalnız servis, 0600) |
| `/var/lib/stremio-subsync/cache/` | Sunulan menülerin cache'i; deploy ve yeniden başlatmada korunur |
| `/usr/local/bin/stremio-subsync` | Program (GitHub sürümünden; `deploy/deploy.sh` kullanılırsa kaynaktan derlenmiş) |
| `/usr/local/bin/subsync` | Sunucu komutu (`deploy/subsync`) |

## Ne loglanmaz

Kurulum token'ı, medya/stream linki, AIOStreams linki, API anahtarları, IMDb
id'si ve altyazı metni hiçbir log'a, dump'a ya da cache dosyasına yazılmaz.
Oynatılan dosyanın adı yalnız token arkasındaki olay panelinde görünür.

- Uygulama: `journalctl -u stremio-subsync` (`subsync logs`).
- Caddy: erişim logu kapalı; URI basan hata logları `deploy/Caddyfile`'da
  devre dışı. Yeni bir `log` direktifi eklenirse token loglara düşer.
- Token yalnız iki yerde, istendiğinde ve o anki terminale basılır: kurulumun
  sonunda ve `subsync link` ile.
