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
| — | Apple TV (Stremio Lite) | — | — | ölçülecek | ölçülecek | ölçülecek |

Ham log satırları:

```
# masaüstü
subtitles request kind=series hints=PlaybackHints { has_filename: true, has_video_size: true, has_video_hash: true }
# Android TV
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
- Apple TV (Stremio Lite) henüz ölçülmedi; cihaza erişim olunca aynı yolla
  ölçülecek. `videoHash` gelmeyebileceği beklentisi orada test edilecek.
