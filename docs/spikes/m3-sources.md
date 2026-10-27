# M3 — Aday altyazı kaynakları

Soru (SS-016): Bir IMDb id + dil için aday altyazılar gerçekten bulunup
indirilebiliyor mu, ve sıralama doğru sürümü öne alıyor mu?

## Kaynaklar

| Kaynak | Anahtar | Liste | İndirme |
|---|---|---|---|
| OpenSubtitles v3 (Stremio add-on'u, `opensubtitles-v3.strem.io`) | yok | `/subtitles/{type}/{id}.json`; dil `lang` (ISO 639-2), `subtitleFileName`, `movieReleaseName`, `fpsMilli`, `season`/`episode` | Doğrudan SRT, sunucu UTF-8'e çeviriyor |
| SubDL (`api.subdl.com/api/v1/subtitles`) | ücretsiz (`SUBDL_API_KEY`) | `imdb_id`, `type=movie|tv`, `languages=TR`, `season_number`/`episode_number`; `release_name`, `url`, `hi` | `dl.subdl.com{url}` → ZIP; bölüm adına uyan SRT seçilir |

SubDL'in dokümanı Cloudflare arkasında; alan adları CloudStream'in SubDL
istemcisinden alındı. Gerçek SubDL denemesi anahtar alınınca yapılacak (SS-017).

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

## Notlar

- OpenSubtitles v3 tek istekte tüm dilleri dönüyor (~90 kayıt); filtre bizde.
- Popüler olmayan filmlerde Türkçe aday az ve çoğu eski TS/CAM sürümü. SubDL
  ikinci kaynak olarak önemli.
- Puanın hizalamaya etkisi SS-014'te ölçülecek: yüksek puanlı aday daha sık
  kabul ediliyor mu?
