# M5 — PGS (resim) İngilizce izden cetvel (SS-025)

Soru: İngilizce altyazısı yalnız PGS olan dosyalarda (kapsama sayımında 21
dosyanın 6'sı) Cues'taki PGS zamanlarından metin cetveli kadar iyi bir cetvel
çıkar mı?

## Hipotez

Cues'ta PGS izlerinin cue noktaları var ama süreleri yok. Hem SRT hem PGS
İngilizce içeren iki dosyada PGS noktası SRT satırının ~2 katı (2008 → 4020,
1061 → 2406). Beklenen: her satır bir "göster" + bir "temizle" display set'i.
Ardışık iki nokta bir satır olur; hangi noktanın satırı açtığı Cues'tan
bilinmez, o yüzden iki faz da denenir.

## Ölçüm düzeni

- `SUBSYNC_DUMP_DIR=dumps` açıkken, eşleşen MKV'de indekslenmiş İngilizce
  PGS izi varsa `dumps/pgs/<unix ms>.json` yazılır. Yalnız zaten okunan
  Cues kullanılır; ek medya isteği yok. Görüntü okunmaz.
- Kayıt: içerik türü, varsa dosyanın İngilizce metin cetvelleri ve her PGS
  izi için iz numarası, forced bayrağı ve cue zamanları (ms, Cues sırasıyla,
  tekrarlar korunur). Kimlik, ad, link, metin yok.
- Kayıt, cetvel olmasa da ve Türkçe aday aranmadan önce alınır; yalnız PGS
  içeren dosyalar da kaydedilir. Menü cache'ten cevaplanan dosyada probe
  çalışmadığından kayıt da çıkmaz: ölçüm için daha önce açılmamış dosya seç
  ya da boş bir `SUBSYNC_CACHE_DIR` ile başlat.
- `cargo run -- pgs-check` (varsayılan `dumps/pgs`) her PGS izi ve faz için
  bir satır basar:

```
<dump> <tür> pgs=<iz> points=<n> repeats=<n> phase=Even|Odd|Each lines=<n>
  median=<ms> long=<%> | text=<iz> lines=<n> ACCEPT|reject fix=<oran><kaydırma> hit=<isabet>
```

| Faz | Satırlar |
|---|---|
| `Even` | Nokta 0–1, 2–3, …: ilk nokta "göster" |
| `Odd` | Nokta 1–2, 3–4, …: ilk nokta "temizle" |
| `Each` | Her nokta bir sonrakine kadar süren satır (temizlemesiz art arda satırlar için) |

`repeats` aynı zamanda iki kez indekslenmiş nokta sayısıdır; eşleme öncesi
tekrarlar teke indirilir. `long` 10 sn'den uzun satırların payıdır.

## Sentetik sonuç (2026-10-29)

45 dakikalık sentetik konuşmadan üretilen metin cetvelinin satır başı ve
sonları PGS noktası yapıldığında `Even` fazı cetvelle `fix=1/1+0ms hit=1.00`
ile kabul ediliyor; `Odd` fazı (satır araları) bu sonucu vermiyor
(`pgs::tests::show_and_clear_pairs_match_the_text_ruler`). Bu yalnız eşleme
kodunu doğrular; gerçek PGS akışında her satırın gerçekten tek "göster" +
tek "temizle" olup olmadığını göstermez.

## Gerçek ölçüm

Bekliyor. Gereken: hem SRT hem PGS İngilizce içeren en az bir dosyayı ve
yalnız PGS içeren bir dosyayı Stremio'da açmak, sonra `pgs-check`.

Tutarsa beklenen: SRT + PGS dosyasında bir fazın (büyük olasılıkla `Even`)
`fix=1/1`, ~0 ms, yüksek isabetle kabulü. Yalnız PGS dosyasında doğru faz
metin cetveli olmadan seçilmek zorunda; bunun için `median` ve `long`
sayıları karşılaştırılır (adım 3'ün girdisi).

| Tarih | Tür | PGS noktası | Tekrar | Faz | Satır | Medyan | Uzun | Metin satırı | Karar | Fix | İsabet |
|---|---|---|---|---|---|---|---|---|---|---|---|
