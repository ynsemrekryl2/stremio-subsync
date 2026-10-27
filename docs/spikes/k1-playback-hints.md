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
| — | Android TV | — | — | ölçülecek | ölçülecek | ölçülecek |
| — | Apple TV (Stremio Lite) | — | — | ölçülecek | ölçülecek | ölçülecek |

Masaüstü ham log satırı:

```
subtitles request kind=series hints=PlaybackHints { has_filename: true, has_video_size: true, has_video_hash: true }
```

## Notlar

- Masaüstünde üç alan da geliyor. Dosya tanıma (K2) için `filename` ile
  `videoSize` yeterli görünüyor.
- İlk ölçüm bir dizi bölümüyle yapıldı. Film (`kind=movie`) ölçümü de
  eklenmeli; farklı çıkması beklenmiyor.
- Android TV ve Apple TV `127.0.0.1`'e erişemez; bu iki satır VPS + HTTPS
  kurulumundan sonra doldurulacak.
