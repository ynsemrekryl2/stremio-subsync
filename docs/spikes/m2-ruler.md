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

| # | İçerik | İz (codec) | cue_refs | with_duration | events | median_ms | overlaps | scale_known |
|---|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | — | — |

## Notlar

- (ölçümden sonra doldurulacak)
