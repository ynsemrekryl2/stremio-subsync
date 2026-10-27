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

| # | Tarih | İçerik | Debrid | Cues boyutu | cue points (video) | İng. metin iz(ler)i | english_text_indexed | Ek istek / bayt |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-10-27 | dizi bölümü | RD | 273 KB | 10553 (282) | ASS forced: 4985 · ASS: 5286 | **evet** | 2 / 273 KB |
| 2 | 2026-10-27 | film (14 altyazı izi) | RD | 505 KB | 19264 (930) | SRT: 1053 · SRT: 1824 (+ PGS: 3648) | **evet** | 2 / 505 KB |
| 3 | 2026-10-27 | film | RD | 312 KB | 13810 (8564) | yok — yalnız PGS (en: 2968) | yok (İng. metin izi yok) | 2 / 312 KB |
| 4 | 2026-10-27 | dizi bölümü | RD | — | — | SRT · SRT | ölçülemedi: `cues: Failed(Read(Other))` | 1 / 0 |
| 5 | 2026-10-27 | dizi bölümü | RD | — | — | altyazı izi yok | ölçülemedi: `cues: Failed(Read(Other))` | 1 / 0 |

MKV olmayan / gerçek dosya olmayan eşleşmeler (K3 sayımına girmez):

| İçerik | hops | ebml_magic | size_matches | content | Yorum |
|---|---|---|---|---|---|
| dizi bölümü (4 istek) | `[Other, Other]` | hayır | hayır | `OtherVideo` | Debrid'de hazır olmayan dosya; muhtemelen "indiriliyor" yer tutucu videosu. Kullanıcı: açılmadı. |
| dizi bölümü (2 istek) | `[Other, RealDebrid]` | hayır | 1. istek hayır, 2. evet | `Other` | MKV değil (muhtemelen MP4) |
| dizi bölümü | `[Other, RealDebrid]` | hayır | evet | `Other` | MKV değil (muhtemelen MP4) |

## Notlar

- **İlk tur (2026-10-27):** Cues'u okunabilen 3 MKV'nin üçünde de altyazı
  izleri indekslenmiş; ASS, SRT ve PGS'in hepsi. İngilizce metin izi olan
  2 dosyanın ikisinde de `english_text_indexed: evet`.
- Cues'ta altyazı noktaları çoğunlukta: 1. dosyada 10553 noktanın yalnız 282'si
  video. Muxer altyazının her bloğunu indekslemiş görünüyor. Bu, cetveli
  doğrudan Cues'tan konum alarak okumayı mümkün kılıyor.
- Cues boyutu 273–505 KB; tek dosya bütçesinin (8 MiB) çok altında. 256 KB'lık
  ilk parça yetmediği için 2 istek gerekti.
- 3. dosyada İngilizce yalnız PGS (görüntü) var. Kararlara göre bu dosya için
  senkronlu altyazı üretilmez.
- 4. ve 5. dosyada Cues okuması `ReadError::Other` ile düştü; hata türü sebebi
  göstermiyordu. Hata ayrıntısı (URL'siz) ve bir kez yeniden deneme eklendi;
  bu dosyalar yeniden ölçülecek. Kullanıcıya göre bazı dosyalar debrid'de hazır
  olmadığı için açılmadı; 4–5'in bunlardan olup olmadığı belli değil.
- Debrid'de hazır olmayan stream'ler de eşleşiyor (K2 `both: 1`), ama range
  okuması gerçek dosya yerine başka bir video döndürüyor. İmzası: `hops`'ta
  Real-Debrid yok, `size_matches: false`, `ebml_magic: false`. Gerçek akışta bu
  durum tanınıp atlanmalı.
