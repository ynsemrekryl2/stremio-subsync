# M3 — Aday altyazı kaynakları

Soru (SS-016): Bir IMDb id + dil için aday altyazılar gerçekten bulunup
indirilebiliyor mu, ve sıralama doğru sürümü öne alıyor mu?

## Kaynaklar

| Kaynak | Anahtar | Liste | İndirme |
|---|---|---|---|
| OpenSubtitles v3 (Stremio add-on'u, `opensubtitles-v3.strem.io`) | yok | `/subtitles/{type}/{id}.json`; dil `lang` (ISO 639-2), `subtitleFileName`, `movieReleaseName`, `fpsMilli`, `season`/`episode` | Doğrudan SRT, sunucu UTF-8'e çeviriyor |
| SubDL (`api.subdl.com/api/v1/subtitles`) | ücretsiz (`SUBDL_API_KEY`) | `imdb_id`, `type=movie|tv`, `languages=TR`, `season_number`/`episode_number`; `release_name`, `url`, `hi` | `dl.subdl.com{url}` → ZIP; bölüm adına uyan SRT seçilir |

SubDL'in dokümanı Cloudflare arkasında; alan adları CloudStream'in SubDL
istemcisinden alındı. Gerçek deneme 2026-10-30'da yapıldı (SS-017, aşağıda).

## Probe

`SUBSYNC_SOURCES_PROBE=1` açıkken her altyazı isteğinde: kaynaklarda ara,
dosya adına göre sırala, en iyi 3 adayı indir, SRT'yi oku. Log satırı
`m3 probe query=… outcome=SourcesProbe { … }` yalnız tür, sayı ve puan taşır.

Sıralama puanı (`src/rank.rs`): dosya adındaki kelimelerin sürüm adında da
geçen payı (en çok 100) + aynı sürüm grubu 40 + aynı kaynak türü (BluRay /
WEB / TV / DVD) 20 + aynı çözünürlük 5. CAM/TS sürümleri −200.

## Sonuçlar (2026-10-27, `tur`, AIOStreams'siz yerel sunucu, elle verilen dosya adlarıyla)

| İçerik | Aday | İlk 5 puan | İndirilen 3 aday: satır sayısı | Not |
|---|---|---|---|---|
| Film A (2024, WEBRip dosyası) | 6 | 165, 165, 70, −160, −160 | 1255, 1262, 1150 | Aynı sürüm iki kez başta |
| Film B (2010, BluRay dosyası) | 7 | 0, −167, −167, −167, −184 | 29, 1456, 1459 | Türkçe adayların hepsi TS/CAM; baştaki 29 satırlık parça (hizalama reddeder) |
| Dizi A bölümü | 2 | 67, 62 | 550, 603 | |
| Dizi B bölümü | 4 | 76, 42, 42, 28 | 524, 593, 675 | İkinci aday ~12 dk daha uzun; farklı kurgu ya da fps |

Her arama + 3 indirme 0.8–1.2 sn sürdü.

## Gerçek oynatma (2026-10-27, M2 ölçümüyle aynı oturum)

Aynı dosyalarda `m3 probe` (yalnız OpenSubtitles, `tur`). Cetvel M2 tablosundan.

| # | Türkçe aday | Puan | İndirilen satır / aralık | Cetvel aralığı | Not |
|---|---|---|---|---|---|
| Film 1 | 0 | — | — | 443–5949 sn | Türkçe yok |
| Film 2 | 1 | 4 | 668 / 636–5361 sn | 592–9585 sn | Film ~160 dk, altyazı ~89 dk'da bitiyor: büyük olasılıkla CD1 |
| Film 3 | 2 | 20, 20 | 746 / 106–4328 sn; 1619 / 1–8660 sn | 89–8666 sn | Biri CD1, diğeri tam; tam olanın sonu cetvelle 6 sn farkla aynı |
| Film 4 | 1 | 40 | 2253 / 7–7613 sn | 4–7631 sn | Uyumlu görünüyor |
| Bölüm 1 | 1 | 10 | 619 / 9–2648 sn | 6–2651 sn | Uyumlu görünüyor |
| Bölüm 2 | 1 | 6 | 211 / 89–1356 sn | — | K2 eşleşmedi, cetvel yok |

- Eski filmlerde **iki parçalı (CD1/CD2) altyazı** yaygın; tek parça şimdilik
  `partial` gibi reddedilir. Birleştirme ayrı iş (SS-019).
- Puanlar düşük (4–40): OpenSubtitles'taki Türkçe sürüm adları dosyaya pek
  benzemiyor. Sıralama yine de CAM/TS'yi geride tutuyor.

## Notlar

- OpenSubtitles v3 tek istekte tüm dilleri dönüyor (~90 kayıt); filtre bizde.
- Popüler olmayan filmlerde Türkçe aday az ve çoğu eski TS/CAM sürümü. SubDL
  ikinci kaynak olarak önemli.
- Puanın hizalamaya etkisi SS-014'te ölçülecek: yüksek puanlı aday daha sık
  kabul ediliyor mu?

## SubDL gerçek denemesi (2026-10-30, SS-017)

Anahtar `scripts/dev.sh subdl` ile Anahtar Zinciri'nde; açılışta
`sources probe: on (lang tur, [OpenSubtitles, SubDl])`.

| İçerik | OpenSubtitles | SubDL | İndirilen 3 aday | Hizalama | Menü |
|---|---|---|---|---|---|
| film (WEB-DL, çok dilli gömülü altyazı, dosya boyutla eşleşti) | 0 aday | 4 aday (puan 80, 75, 60, 30) | 3 ZIP'in üçünden de SRT okundu, 1768 satır | en-GB / en-US izlerinde kabul: fix 1/1, 0–3 ms, isabet %94 / %97 / %98 | 2 girdi (3 kabulden kopyalar teke indi); Stremio altyazıyı çekti |

- OpenSubtitles'ta Türkçesi hiç olmayan bir film SubDL sayesinde ilk
  açılışta senkron Türkçe altyazı aldı; SubDL olmasa "Aday yok" → İngilizce
  yedek yoluna düşecekti.
- Adayların satır sayısı (1768) dosyadaki gömülü Türkçe izin cue sayısıyla
  aynı: SubDL'deki altyazı büyük olasılıkla bu sürümden çıkarılmış. Bu tür
  dosyalarda Stremio gömülü Türkçe izi zaten kendisi gösterir.
- Metin izleri 3 ve 4'te (SDH, farklı satır bölme) kabul yok; yerel geçiş de
  %68–72'de kaldı. İki iz yeterliyse kabul için bir iz yetiyor, sorun değil.
- Aynı oynatma için Stremio iki istek gönderdi (`videoHash` önce yok, sonra
  var); iki probe paralel koştu ve aynı sonucu verdi. Çift iş, ama doğruluk
  sorunu değil.

## Paralel dalın ek ölçümü

`claude/m3e-basha-a4d3a4` dalındaki API alanları ve 6 başlık / 15 SRT
indirme ölçümü [`m3-sources-legacy.md`](m3-sources-legacy.md) içinde korunur.
Bu, farklı sıralama koduyla alınmış tarihsel bir örneklemdir; puanları bu
dosyadaki güncel puanlarla karşılaştırılmaz.
