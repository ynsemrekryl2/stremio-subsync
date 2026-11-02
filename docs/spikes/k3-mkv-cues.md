# K3 — Range desteği ve Cues'ta altyazı indeksi

Soru: Debrid linkleri Range isteğini destekliyor mu? Gerçek MKV'lerde
İngilizce metin altyazı izi Cues'ta indeksleniyor mu? (en az 10 dosya)

## Range desteği

Real-Debrid: **evet**. Ayrıntı ve ham satırlar K2 notunda
(`docs/spikes/k2-aiostreams-match.md`, "Range okuma sonuçları"). Okumalar
oynatmayla aynı ev IP'sinden yapıldı.

**VPS IP'sinden (2026-10-30, SS-008): evet.** Oracle sunucusu
Stremio oynatması evden sürerken aynı filmi okudu: AIOStreams 14 stream,
`both: 1` eşleşme (0,5 sn); dosyanın Tracks/Cues'u ve iki PGS izinin cetveli
(2087 ve 2044 olay) okundu; 4 deneme (2 aday × 2 iz) 40–45 ms; istekten
menüye ~7 sn. Debrid sunucu IP'sinden gelen okumayı reddetmedi, oynatma
kesilmedi. İstek tekrarında (Stremio aynı isteği iki kez attı) okuma yine
sorunsuz.

**TorBox (2026-10-31, SS-007): evet**, VPS IP'sinden. 206, `Accept-Ranges`,
boyut tutuyor, `Content-Type: video/x-matroska`. Ayrıntı K2 notunda.

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
| 4 | 2026-10-27 | dizi bölümü | RD | — | — | SRT · SRT | yeniden ölçülmedi (dosya sonu hatası) | 1 / 0 |
| 5 | 2026-10-27 | dizi bölümü | RD | — | — | altyazı izi yok | yeniden ölçülmedi; İng. metin izi zaten yok | 1 / 0 |
| 6 | 2026-10-27 | dizi bölümü (31 altyazı izi) | RD | 855 KB | 31603 (1785) | SRT: 908 · SRT: 963 | **evet** | 2 / 855 KB |
| 7 | 2026-10-27 | film | RD | 458 KB | 19121 (1345) | SRT forced: 73 · SRT: 1769 (+ PGS: 2626) | **evet** | 2 / 458 KB |
| 8 | 2026-10-27 | dizi bölümü (30 iz) | RD | 395 KB | 16265 (1744) | SRT: 414 (+ PGS: 734) | **evet** | 2 / 395 KB |
| 9 | 2026-10-27 | dizi bölümü (32 iz) | RD | 352 KB | 14287 (1732) | SRT: 321 · SRT: 463 (+ PGS: 658) | **evet** | 2 / 352 KB |
| 10 | 2026-10-27 | film | RD | 113 KB | 4480 (1352) | SRT: 1014 · SRT: 1166 | **evet** | 1 / 116 KB |
| 11 | 2026-10-27 | dizi bölümü | RD | 18 KB | 871 (871) | altyazı izi yok | yok (İng. metin izi yok) | 1 / 20 KB |
| 12 | 2026-10-27 | dizi bölümü | RD | — | — | yok — yalnız PGS | yeniden ölçülmedi; İng. metin izi zaten yok | 1 / 0 |
| 13 | 2026-10-27 | dizi bölümü | RD | 71 KB | 2809 (874) | SRT: 771 (+ pl ASS: 589) | **evet** | 1 / 73 KB |
| 14 | 2026-10-27 | dizi bölümü (14 iz) | RD | 160 KB | 6109 (709) | SRT: 475 | **evet** | 1 / 167 KB |
| 15 | 2026-10-27 | dizi bölümü | RD | 30 KB | 1229 (678) | SRT: 551 | **evet** | 1 / 30 KB |
| 16 | 2026-10-27 | film | RD | 59 KB | 2715 (2715) | SRT: **0** (fr-CA, es-419 SRT: 0) | **hayır** — Cues yalnız video | 1 / 60 KB |
| 17 | 2026-10-27 | dizi bölümü | RD | 22 KB | 896 (369) | SRT: 527 | **evet** | 1 / 23 KB |
| 18 | 2026-10-31 | film | TorBox | 118 KB | 4969 (2837) | SRT: 1132 (+ pl SRT: 1000) | **evet** | 1 / 119 KB |

MKV olmayan / gerçek dosya olmayan eşleşmeler (K3 sayımına girmez):

| İçerik | hops | ebml_magic | size_matches | content | Yorum |
|---|---|---|---|---|---|
| dizi bölümü (4 istek) | `[Other, Other]` | hayır | hayır | `OtherVideo` | Debrid'de hazır olmayan dosya; muhtemelen "indiriliyor" yer tutucu videosu. Kullanıcı: açılmadı. |
| dizi bölümü (2 istek) | `[Other, RealDebrid]` | hayır | 1. istek hayır, 2. evet | `Other` | MKV değil (muhtemelen MP4) |
| dizi bölümü | `[Other, RealDebrid]` | hayır | evet | `Other` | MKV değil (muhtemelen MP4) |
| dizi bölümü (3. tur) | `[Other, RealDebrid]` | hayır | evet | `Other` | MKV değil (muhtemelen MP4); aynı dizinin 2. turdaki bölümleri de MKV değildi |

## Sonuç (Real-Debrid, 2026-10-27)

- **Range:** destekleniyor (206), ev IP'sinden.
- **İndeksleme:** Cues'u okunan ve İngilizce metin izi olan **12 dosyanın
  11'inde** iz Cues'ta indekslenmiş (~%92). İndekslenmeyen tek dosyada (16)
  Cues yalnız video anahtar karelerini içeriyor; o dosyadaki hiçbir altyazı izi
  indekslenmemiş. Muxer ayarı dosya bazında, iz bazında değil.
- İngilizce metin izi olmayan dosyalar: 3 ve 12 yalnız PGS, 5 ve 11 hiç
  altyazısız. Bunlar için zaten cetvel yok.
- **Maliyet:** Cues 18–855 KB; ilk 64 KB'tan sonra 1–2 istek, 0,35–1,3 sn.
  8 MiB'lık bütçeye hiç yaklaşılmadı.
- **Sonuç:** Planın varsayımı (Cues'tan hedefli okuma) Real-Debrid'deki MKV'lerin
  büyük çoğunluğu için geçerli. İndekslenmemiş dosyalar için (12'de 1) ya
  desteklenmez denir, ya da ileride küme taraması gibi pahalı bir yedek yol
  düşünülür.
- Kullanıcı 3. turun ortasında bir dosyada debrid hatası aldı. Logda buna ait
  ayrı bir satır yok: ya Stremio altyazı istemedi, ya da hata yalnız oynatıcı
  tarafında oldu. Sayılan her satırda EBML başlığı, Tracks ve Cues dosyanın
  kendisinden okunduğu için sayımı etkilemiyor.
- Açık kalanlar: TorBox'ta 10 dosya (SS-007; 2026-10-31 itibarıyla 1/10,
  o da indekslenmiş). VPS IP'sinden okuma SS-008 ile doğrulandı.

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
- **İkinci tur (2026-10-27):** Okunabilen 4 dosyanın dördünde de İngilizce
  metin izi indekslenmiş. Toplam: Cues'u okunan ve İngilizce metin izi olan
  **6/6 dosyada indekslenmiş**.
- **Dosya sonu hatası:** 4, 5 ve 10–16 numaralı dosyalarda Cues okuması
  `end of file before message length reached` ile düştü; yeniden deneme de aynı.
  Sebep: 256 KB'lık okuma dosya sonunu aşıyordu. Real-Debrid böyle bir istekte
  aralığı kısaltmak yerine istenen uzunluğu bildiriyor, eksik gönderip bağlantıyı
  kapatıyor. Düzeltme: ilk yanıttaki toplam boyutla okumalar dosya sonunda
  kesiliyor. Yerel sunucuda aynı davranış taklit edilerek hata tekrarlandı ve
  düzeltmenin çözdüğü görüldü. Bu dosyalar yeniden ölçülecek.
- Başarısız grubun altyazısı indekslenmemiş dosyalar olduğu tahmini **yanlış
  çıktı**. Yeniden ölçülen 6 dosyanın 5'inde İngilizce metin izi indekslenmiş;
  Cues yalnız dosyalar kısa olduğu için küçüktü (dizi bölümleri, 18–160 KB).
- Debrid'de hazır olmayan stream'ler de eşleşiyor (K2 `both: 1`), ama range
  okuması gerçek dosya yerine başka bir video döndürüyor. İmzası: `hops`'ta
  Real-Debrid yok, `size_matches: false`, `ebml_magic: false`. Gerçek akışta bu
  durum tanınıp atlanmalı.
