# stremio-subsync

Kişisel kullanım için bir Stremio altyazı add-on'u. Oynatılan dosyanın
**içindeki İngilizce metin altyazısını** cetvel olarak kullanır, dış
kaynaklardan bulunan altyazıları bu cetvele kaydırır ve yalnız filmin başında,
ortasında ve sonunda aynı kaydırmayla tutanları sunar.

Ses analizi yapmaz. Filmi indirmez veya proxy'lemez; dosyanın yalnız
içindekiler tablosunu HTTP Range ile okur. Blok okuma denemesi ayrıca
açılır (SS-028); otomatik çeviri henüz uygulanmadı.

## Durum

Çalışıyor (yerel): oynatılan MKV'nin İngilizce altyazısından cetvel çıkarır,
OpenSubtitles (ve anahtar varsa SubDL) adaylarını hizalar, baş / orta / son
testini geçenleri kaydırılmış olarak Stremio'ya sunar ve sonucu cache'ler.
İlk açılışta yanıt birkaç saniye sürebilir (en çok 15 sn); sonraki açılışlar
cache'ten gelir. Altyazı üretilemezse menüde nedeni yazan tek bir "SubSync: …"
girdisi görünür (örneğin "İngilizce iz yok"). VPS kurulumu henüz yapılmadı (SS-006).
Plan: [`docs/plan.md`](docs/plan.md) · Kararlar: [`docs/decisions.md`](docs/decisions.md)
· İş listesi: [`TASKS.md`](TASKS.md)

## Çalıştırma

Yerelde (macOS) tek komut; Finder'da `scripts/SubSync.command`'a çift
tıklamak da aynısını yapar:

```bash
scripts/dev.sh
```

İlk çalıştırmada token üretilir, AIOStreams linki (gizli girişle) sorulur,
ikisi macOS Anahtar Zinciri'ne kaydedilir ve kurulum linki panoya kopyalanır.
Sonraki açılışlar hiçbir şey sormaz; kurulum linki aynı kalır, eklentiyi
yeniden kurmak gerekmez. `scripts/dev.sh link` linki yeniden panoya kopyalar,
`scripts/dev.sh dashboard` olay panelini açar, `scripts/dev.sh forget` iki
sırrı siler (sonraki açılışta yeni token). Ek `SUBSYNC_*` değişkenleri
geçer, örneğin `SUBSYNC_DUMP_DIR=dumps scripts/dev.sh`.

Elle çalıştırmak için:

```bash
export SUBSYNC_TOKEN="$(openssl rand -hex 24)"   # en az 32 karakter
cargo run
```

Stremio'ya kurulacak link:

```
http://127.0.0.1:7001/<SUBSYNC_TOKEN>/manifest.json
```

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
| `SUBDL_API_KEY` | — | SubDL'in ücretsiz anahtarı; yoksa yalnız OpenSubtitles kullanılır. Loglanmaz. |
| `SUBSYNC_DUMP_DIR` | — | Verilirse hizalanan her isteğin zamanları (yalnız sayılar) buraya JSON olarak yazılır; `cargo run -- calibrate <dizin>` bunları yeniden hizalar. |
| `SUBSYNC_LABEL_MODE` | kapalı | `1` ise indirilen her aday Stremio'nun altyazı menüsünde "SubSync 2 ✓ +1.60s · 7530" gibi görünür: sıra, kabul/ret, kaydırma, dump eki. Kaydırması olan her adayın bir de `yerel` sürümü gelir (SS-021: kayma filmin içinde değişiyorsa satırları çevresine göre ayrıca kaydırır). Etiketleme için; istek 15 sn'ye kadar bekleyebilir. |

Sunucu kurulumu: [`docs/deploy.md`](docs/deploy.md)

## Test

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
