# K2 — AIOStreams stream listesinde eşleşme

Soru: Stremio'nun altyazı isteğinde gönderdiği `filename` + `videoSize` ile
AIOStreams stream listesinden tek bir stream'i kesin seçebiliyor muyuz?
Seçilen stream'in URL'si ne tür bir hedef?

## Yöntem

- Sunucu `SUBSYNC_AIOSTREAMS_URL` (AIOStreams manifest linki) ile başlatılır.
  Link hiçbir yerde loglanmaz; açılışta yalnız `aiostreams probe: on` yazar.
- Her altyazı isteğinde, yanıtı bekletmeden arka planda
  `…/stream/{type}/{id}.json` okunur ve oynatıcının ipuçlarıyla karşılaştırılır.
- **Stream URL'lerine istek atılmaz** (TorBox'ta indirme başlatabilir). URL'yi
  çağırmanın debrid tarafında ne tetiklediği ayrı ve kullanıcı onayıyla denenecek.
- Log satırı yalnız sayılar ve küçük etiketler taşır:

```
k2 probe kind=movie outcome=Matched { elapsed_ms: …, report: MatchReport {
  streams, with_filename, with_size,
  filename_exact, filename_loose, size, both,
  target: Some(Http { same_host }) | Some(InfoHashOnly) | Some(Missing) | None } }
```

| Alan | Anlamı |
|---|---|
| `streams` | Listedeki stream sayısı |
| `with_filename` / `with_size` | `behaviorHints` içinde filename / videoSize taşıyanlar |
| `filename_exact` | Dosya adı birebir aynı olanlar |
| `filename_loose` | Büyük-küçük harf ve klasör yolu yok sayılınca aynı olanlar |
| `size` | Boyutu birebir aynı olanlar |
| `both` | Dosya adı ve boyut birlikte aynı olanlar — K2'nin asıl cevabı; 1 olmalı |
| `target` | Yalnız `both == 1` iken: URL AIOStreams'in kendi host'unda mı (`same_host: true`, proxy/playback yolu) yoksa başka host'ta mı |

## Çalıştırma

Sunucu ve Funnel açılırken ortamda `SUBSYNC_AIOSTREAMS_URL` de verilir; sonra
masaüstünde ve TV'de birkaç film/bölüm açılır, `k2 probe` satırları aşağıya
eklenir. Tercihen hem Real-Debrid hem TorBox'tan gelen stream'ler denenir.

## Sonuçlar

| Tarih | İstemci | İçerik | Debrid | streams | both | filename_exact | size | target | elapsed_ms |
|---|---|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | — | — | — |

## Notlar

- (ölçümden sonra doldurulacak)
