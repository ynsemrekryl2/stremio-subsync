# K3 — Range desteği ve Cues'ta altyazı indeksi

Soru: Debrid linkleri Range isteğini destekliyor mu? Gerçek MKV'lerde
İngilizce metin altyazı izi Cues'ta indeksleniyor mu? (en az 10 dosya)

## Range desteği

Real-Debrid: **evet**. Ayrıntı ve ham satırlar K2 notunda
(`docs/spikes/k2-aiostreams-match.md`, "Range okuma sonuçları"). Okumalar
oynatmayla aynı ev IP'sinden yapıldı; VPS IP'sinden okuma henüz denenmedi.
TorBox: SS-007.

## Yöntem

`SUBSYNC_RANGE_PROBE=1` açıkken, range okuması 206 + EBML başlığıyla dönerse
aynı dosyada `mkv::inspect` çalışır:

1. İlk 64 KB'tan EBML ve Segment başlığı, SeekHead, ve sığıyorsa Tracks okunur.
2. Tracks ilk 64 KB'ta değilse SeekHead'deki konumundan okunur.
3. Cues, SeekHead'deki konumundan okunur. Cues kaydı yoksa ikinci bir SeekHead
   aranır.
4. Bütçe: ilk okumadan sonra en çok **6 istek** ve **8 MiB**. Cues daha büyükse
   okunmaz; yalnız boyutu `TooLarge { bytes }` olarak raporlanır.

İz adı ve altyazı metni okunmaz ve loglanmaz. Log'da `k2 probe` satırının
sonuna `mkv: Some(Ok(MkvReport { … }))` eklenir:

| Alan | Anlamı |
|---|---|
| `requests` / `bytes_read` | İlk 64 KB'tan sonra yapılan ek okumalar |
| `subtitles` | Her altyazı izi: numara, codec (`Srt` / `Ass` / `WebVtt` / `Pgs` / `VobSub` / …), dil kodu, `default`, `forced`, `cue_points` |
| `cue_points` | Bu izin konumunu taşıyan cue point sayısı; 0 = iz indekslenmemiş |
| `cues` | `Read { bytes, points, video_points }` / `NotReferenced` / `TooLarge { bytes }` / `Failed(…)` |
| `english_text_indexed` | Dosya için K3 cevabı: İngilizce metin izi Cues'ta var mı (`None` = İngilizce metin izi yok) |

Yerel deneme (2026-10-27): ffmpeg ile üretilmiş 2 MB, 120 sn'lik MKV
(H.264 + İngilizce ve Türkçe SRT), Range destekleyen yerel sunucu. Sonuç:
1 ek istek, 1536 bayt; iki SRT izinin de `cue_points: 30` (ffmpeg altyazı
izlerini indeksliyor), `english_text_indexed: Some(true)`.

## Sonuçlar

Her satır bir dosya. `İng. metin iz(ler)i` sütunu codec ve `cue_points`.

| Tarih | İçerik | Debrid | Cues boyutu | cue points (video) | İng. metin iz(ler)i | english_text_indexed | Ek istek / bayt |
|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | — |

## Notlar

- (ölçümden sonra doldurulacak)
