# M2 — Cetvel Cues'tan çıkar mı?

Soru (SS-009): Altyazı izlerinin cue noktaları `CueDuration` taşıyor mu? Taşıyorsa
her İngilizce satırın başlangıcı (`CueTime`) ve süresi zaten okunan Cues'ta
var; cetvel **ek okuma olmadan** çıkar. Taşımıyorsa süreler blokların
kendisinden okunmalı (olay başına bir Range isteği; bölüm başına ~900 istek,
çok pahalı).

## Yöntem

`SUBSYNC_RANGE_PROBE=1` açıkken K3 okuması aynen yapılır; ek olarak:

- Info'dan `TimestampScale` okunur (yoksa Matroska varsayılanı 1 ms).
- Her altyazı izi için `cue_durations`: `CueDuration` taşıyan cue noktası sayısı.
- Cetvel izi seçilir: İngilizce, metin, indekslenmiş; mümkünse forced olmayan,
  sonra cue noktası en çok olan.
- `ruler: Some(RulerReport { … })` loglanır. Yalnız sayılar ve süreler; metin
  okunmaz.

| Alan | Anlamı |
|---|---|
| `track` | Seçilen iz numarası |
| `cue_refs` / `with_duration` / `with_relative` | İzin cue noktaları; `CueDuration` ve `CueRelativePosition` taşıyanlar |
| `events` | Süreli cue noktalarından çıkan farklı olay sayısı |
| `first_ms` / `last_end_ms` | İlk satırın başlangıcı, son satırın bitişi |
| `median_ms` | Olay süresi ortancası |
| `overlaps` | Önceki satır bitmeden başlayan satırlar (iki satırlı/ASS katmanları) |

Yerel deneme (2026-10-27): ffmpeg MKV, 30 satırlık SRT (her biri 2 sn, 1. sn'den
başlayıp 4 sn arayla). Sonuç: `cue_refs: 30, with_duration: 30, events: 30,
first_ms: 1000, last_end_ms: 119000, median_ms: 2000, overlaps: 0` — SRT ile
birebir aynı.

## Sonuçlar

Gerçek oynatma, 2026-10-27, masaüstü Stremio 5.1.30 + AIOStreams + Real-Debrid.

| # | İçerik | İz (codec) | cue_refs | with_duration | events | median_ms | overlaps | scale_known |
|---|---|---|---|---|---|---|---|---|
| 1 | Film, ~99 dk | 4 (ASS; tek İngilizce iz, "forced" işaretli) | 1107 | 1107 | 1107 | 1435 | 0 | evet |
| 2 | Film, ~160 dk | 10 (SRT) | 1061 | 1061 | 1061 | 2294 | 0 | evet |
| 3 | Film, ~144 dk | 3 (SRT) | 2008 | 2008 | 2008 | 2335 | 0 | evet |
| 4 | Film, ~127 dk | 5 (SRT; iki İngilizce izden satırı çok olan) | 2356 | 2356 | 2356 | 1919 | 0 | evet |
| 5 | Dizi bölümü, ~44 dk | 5 (SRT) | 1006 | 1006 | 1006 | 2002 | 0 | evet |

Okuma maliyeti (Cues dahil, ilk 64 KB'tan sonra):

| # | Okunan | İstek | Süre | Dosyadaki altyazı izi |
|---|---|---|---|---|
| 1 | 44 KB | 1 | 0.45 sn | 1 |
| 2 | 188 KB | 1 | 0.53 sn | 6 (3'ü PGS) |
| 3 | 625 KB | 2 | 1.1–1.3 sn | 7 (6'sı PGS) |
| 4 | 1.28 MB | 2 | 1.3–1.4 sn | 31 SRT |
| 5 | 74 KB | 1 | 0.48 sn | 2 |

## Notlar

- **SS-009 cevabı: evet.** 5 dosyanın 5'inde metin altyazı izlerinin her cue
  noktası `CueDuration` taşıyor (`with_duration` = `cue_refs`). Cetvel ek
  okuma olmadan Cues'tan çıkıyor; altyazı blokları okunmuyor.
- Diğer metin izleri de aynı: tüm SRT/ASS izlerinde `cue_durations` =
  `cue_points`. PGS izlerinde 0; zaten kullanılmıyorlar.
- Maliyet Cues boyutuyla büyüyor; 31 altyazı izli dosyada 1.28 MB, 2 istek.
  Bütçe gerekirse yalnız İngilizce izin cue noktalarını tutacak şekilde
  okuma sınırlanabilir; şimdilik sorun değil.
- Dosya 4'te iki İngilizce SRT var (2068 ve 2356 satır); seçim kuralı satırı
  çok olanı aldı, büyük olasılıkla SDH. SDH'deki ses efekti satırları
  hizalamada gürültü olabilir; SS-014'te iki izle ayrı ayrı denenmeli.
- Dosya 1'in tek İngilizce izi "forced" işaretli ama 1107 satır; işaret
  güvenilir değil. Tek iz olduğu için yine seçildi, doğru davranış.
- Hiçbir izde `overlaps` yok; ASS izinde de satırlar üst üste binmiyor.

## Paralel M2 kaydı (673f3ff)

Aşağıdaki ayrı kayıt `claude/devam-edelim-2bac6b` dalından korunmuştur.
Ana tabloyla aynı dosyalar bulunabilir; iki örneklem toplanarak “10 farklı
dosya” denmez. Bu not `with_relative == cue_refs` gözlemini de içerir;
blok okuma maliyeti henüz ölçülmemiştir (SS-028).

### Ölçüm

| # | İçerik | İz (codec) | cue_refs | with_duration | events | median_ms | overlaps | scale_known |
|---|---|---|---|---|---|---|---|---|
| 1 | film, RD | 4 (SRT) | 1769 | 1769 | 1769 | 2263 | 0 | evet |
| 2 | film, RD | 3 (SRT) | 2110 | 2110 | 2110 | 1877 | 0 | evet |
| 3 | film, RD | 3 (SRT) | 2008 | 2008 | 2008 | 2335 | 0 | evet |
| 4 | film, RD | 10 (SRT) | 1061 | 1061 | 1061 | 2294 | 0 | evet |
| 5 | film, RD | 4 (ASS) | 1107 | 1107 | 1107 | 1435 | 0 | evet |

5 farklı dosya (1. ve 3. dosya birer kez daha açıldı, aynı sayılar çıktı; bir
açılışta eşleşme olmadı, ölçüm yok). Cetvel için ek okuma: yok; Cues okuması
43–625 KB, 1–2 istek, ~0,4–1,2 sn.

### Notlar

- **SS-009: evet.** Metin izlerinde `with_duration == cue_refs` ve `with_relative
  == cue_refs`, 5/5 dosya. Cetvel yalnız Cues'tan çıkıyor; blok okumak gerekmiyor.
- PGS (görüntü) izlerinde `CueDuration` yok (`cue_durations: 0`); zaten cetvel
  adayı değil, seçici metin izlerini alıyor.
- Seçici doğru çalıştı: forced olmayan İngilizce metin izi tercih edildi
  (dosya 1'de forced iz 3 yerine iz 4).
- `overlaps: 0` ASS dahil her dosyada; iki satırlı/katmanlı çakışma bu örneklemde yok.
- Ortanca olay süresi 1,4–2,3 sn; olay sayısı 1000–2100 aralığında.
- Sınırlama: yalnız Real-Debrid, yalnız film; dizi ve TorBox ölçülmedi.
