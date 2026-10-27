# stremio-subsync

Kişisel kullanım için bir Stremio altyazı add-on'u. Oynatılan dosyanın
**içindeki İngilizce metin altyazısını** cetvel olarak kullanır, dış
kaynaklardan bulunan altyazıları bu cetvele kaydırır ve yalnız filmin başında,
ortasında ve sonunda aynı kaydırmayla tutanları sunar.

Ses analizi yapmaz. Filmi indirmez veya proxy'lemez; dosyanın yalnız
içindekiler tablosunu ve altyazı bloklarını HTTP Range ile okur.

## Durum

İskelet: manifest ve boş altyazı yanıtı dönen, gizli token'lı HTTP sunucusu.
Plan: [`docs/plan.md`](docs/plan.md) · Kararlar: [`docs/decisions.md`](docs/decisions.md)
· İş listesi: [`TASKS.md`](TASKS.md)

## Çalıştırma

```bash
export SUBSYNC_TOKEN="$(openssl rand -hex 24)"   # en az 32 karakter
cargo run
```

Stremio'ya kurulacak link:

```
http://127.0.0.1:7000/<SUBSYNC_TOKEN>/manifest.json
```

| Değişken | Varsayılan | Açıklama |
|---|---|---|
| `SUBSYNC_TOKEN` | — (zorunlu) | Kurulum linkindeki gizli parça. Linki bilen sunucuyu kullanır. |
| `SUBSYNC_BIND` | `127.0.0.1:7000` | Dinlenen adres. VPS'te HTTPS reverse proxy'nin arkasında kalır. |
| `SUBSYNC_LANG` | `tur` | Aranan altyazı dili (ISO 639-2, OpenSubtitles adlarıyla: `tur`, `eng`, `pob`). |
| `SUBSYNC_SOURCES_PROBE` | kapalı | `1` ise her istekte aday altyazılar aranır ve en iyi 3'ü indirilir (M3 ölçümü). |
| `SUBDL_API_KEY` | — | SubDL'in ücretsiz anahtarı; yoksa yalnız OpenSubtitles kullanılır. Loglanmaz. |
| `SUBSYNC_DUMP_DIR` | — | Verilirse hizalanan her isteğin zamanları (yalnız sayılar) buraya JSON olarak yazılır; `cargo run -- calibrate <dizin>` bunları yeniden hizalar. |
| `SUBSYNC_LABEL_MODE` | kapalı | `1` ise indirilen her aday Stremio'nun altyazı menüsünde "SubSync 2 ✓ +1.60s · 7530" gibi görünür: sıra, kabul/ret, kaydırma, dump eki. Etiketleme için; istek 15 sn'ye kadar bekleyebilir. |

Sunucu kurulumu: [`docs/deploy.md`](docs/deploy.md)

## Test

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
