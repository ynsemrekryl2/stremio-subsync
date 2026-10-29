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

2026-10-29: hem SRT hem PGS İngilizce içeren iki dizi bölümü ve bir film
(Real-Debrid, masaüstü Stremio). Yalnız PGS içeren dosya henüz açılmadı.

Tutarsa beklenen: SRT + PGS dosyasında bir fazın (büyük olasılıkla `Even`)
`fix=1/1`, ~0 ms, yüksek isabetle kabulü. Yalnız PGS dosyasında doğru faz
metin cetveli olmadan seçilmek zorunda; bunun için `median` ve `long`
sayıları karşılaştırılır (adım 3'ün girdisi).

| Tarih | Tür | PGS noktası | Tekrar | Faz | Satır | Medyan | Uzun | Metin satırı | Karar | Fix | İsabet |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-29 | dizi A, iz 6 | 1174 | 0 | Even | 587 | 2419 ms | %0 | 576 / 576 | ACCEPT | 1/1 +0 ms | 1.00 |
| 2026-10-29 | dizi A, iz 6 | 1174 | 0 | Odd | 586 | 292 ms | %5 | 576 | reject | — | 0.08 |
| 2026-10-29 | dizi A, iz 6 | 1174 | 0 | Each | 1173 | 1752 ms | %2 | 576 / 576 | ACCEPT | 1/1 +0 ms | 1.00 |
| 2026-10-29 | dizi A, iz 7 | 1553 | 0 | en iyi: Odd | 776 | 1919 ms | %1 | 576 | reject (Disagree) | 1/1 +83 ms | 0.82 |
| 2026-10-29 | dizi A, iz 22 | 2467 | 0 | Even | 1233 | 2085 ms | %0 | 576 | reject | — | 0.24 |
| 2026-10-29 | dizi B, iz 5 | 1104 | 0 | Even | 552 | 1920 ms | %0 | 546 / 523 | ACCEPT | 1/1 +0 ms | 1.00 |
| 2026-10-29 | dizi B, iz 5 | 1104 | 0 | Odd | 551 | 160 ms | %2 | 546 | reject | — | 0.21 |
| 2026-10-29 | dizi B, iz 5 | 1104 | 0 | Each | 1103 | 1280 ms | %1 | 546 | ACCEPT (yalnız 1 iz) | 1/1 +0 ms | 1.00 |
| 2026-10-29 | film C, iz 6 | 4148 | 0 | Even | 2074 | 1835 ms | %0 | 2137 / 1358 | ACCEPT | 1/1 +7 ms / −21 ms | 0.86 / 0.84 |
| 2026-10-29 | film C, iz 6 | 4148 | 0 | Odd | 2073 | 84 ms | %1 | 2137 / 1358 | reject | — | 0.22 |
| 2026-10-29 | film C, iz 6 | 4148 | 0 | Each | 4147 | 1209 ms | %0 | 2137 / 1358 | reject | — | 0.31 / 0.34 |

Yorum:

- **Hipotez tuttu.** Her dosyanın ilk İngilizce PGS izinde nokta sayısı metin
  satırının ~2 katı; `Even` fazı (nokta 0–1 = göster–temizle) aynı dosyanın
  iki metin cetveliyle de `fix=1/1 +0 ms`, isabet 1.00 veriyor. PGS zamanları
  metin satırlarıyla birebir aynı; tekrar yok.
- **Faz metin olmadan seçilebilir:** `Even` satır medyanı 1.9–2.4 sn, uzun
  satır %0; `Odd` (satır araları) medyanı 160–292 ms. Medyanı büyük olan faz
  doğru faz. `Each` dizilerde kabul ediliyor ama filmde (C) ret aldı; adım 3
  için doğru faz `Even`.
- **Filmde de tutuyor:** C'nin PGS izi (2074 satır) iki farklı metin iziyle
  (2137 ve 1358 satır, biri muhtemelen SDH) ~0 ms ile kabul; isabet 0.84–0.86,
  çünkü izlerin satır bölümlemesi birebir aynı değil.
- **Her İngilizce PGS izi aynı değil:** A dosyasındaki 7 ve 22 numaralı
  izler (1553 ve 2467 nokta) metin cetveliyle tutmuyor; muhtemelen SDH,
  yorum ya da farklı yapı. Kabul testi zaten yanlış cetveli eliyor, bu yüzden
  adım 3'te her İngilizce PGS izi `Even` fazıyla ayrı bir cetvel olarak
  denenebilir (metin cetvellerinde olduğu gibi).
