# K1 — Oynatıcıların gönderdiği alanlar

Soru: Altyazı isteğinde `filename`, `videoSize` ve `videoHash` alanlarından
hangileri geliyor?

Yöntem: Add-on, her istekte yalnız alanın var olup olmadığını loglar
(`subtitles request kind=… hints=PlaybackHints { has_…: bool }`). Değerlerin
kendisi loglanmaz.

## Sonuçlar

| Tarih | İstemci | Sürüm | İçerik | filename | videoSize | videoHash |
|---|---|---|---|---|---|---|
| 2026-10-27 | Stremio masaüstü, macOS 27.2 | 5.1.30 | dizi bölümü | var | var | var |
| 2026-10-27 | Stremio Android TV | kaydedilmedi | film | var | var | var |
| 2026-10-30 | Stremio Apple TV 4K | kaydedilmedi | film | var | var | var |

Ham log satırları:

```
# masaüstü
subtitles request kind=series hints=PlaybackHints { has_filename: true, has_video_size: true, has_video_hash: true }
# Android TV
subtitles request kind=movie hints=PlaybackHints { has_filename: true, has_video_size: true, has_video_hash: true }
# Apple TV 4K
subtitles request kind=movie hints=PlaybackHints { has_filename: true, has_video_size: true, has_video_hash: true }
```

TV ölçümleri Mac'teki sunucuya Tailscale Funnel üzerinden (genel HTTPS)
yapıldı. Add-on masaüstüne kuruldu, Stremio hesabı üzerinden TV'ye geçti.

## Notlar

- Masaüstünde üç alan da geliyor. Dosya tanıma (K2) için `filename` ile
  `videoSize` yeterli görünüyor.
- Android TV'de de üç alan geliyor. `kind=movie` isteği de aynı alanları taşıyor.
- Masaüstünde aynı bölüm için iki istek geldi: ilkinde `videoHash` **yok**,
  ikincisinde var (2026-10-27, K2 ölçümü sırasında). Hash ilk istekte
  hazır olmayabiliyor; dosya tanıma hash'e dayanmamalı.
- Apple TV 4K'da da üç alan geliyor (2026-10-30); `videoHash` gelmeyebilir
  beklentisi doğrulanmadı. Oynatma tek istek üretti, istek masaüstündekiyle
  aynı yoldan geçti: AIOStreams eşleşmesi `Both`, RD'den 206 Range, MKV
  Cues okundu. Film yalnız PGS İngilizce içeriyordu; PGS cetveli (1267 satır)
  çıktı, 2 OpenSubtitles adayı pencereler arası kaydırma tutmadığı için
  reddedildi → menüde `NoneAccepted` ("uyan altyazı yok") tek girdi sunuldu.
- Mac'te 7000 portunu macOS AirPlay Alıcısı (ControlCenter) tuttuğu için
  sunucu `SUBSYNC_BIND=127.0.0.1:7001` ile açıldı; Funnel 443 → 7001.
