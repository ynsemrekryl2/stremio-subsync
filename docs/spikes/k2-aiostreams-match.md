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
  filename_exact, filename_loose, size, both, matched_by,
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
| `matched_by` | Seçilen stream: `Some(Both)` ad + boyut, `Some(Size)` yalnız boyut (SS-020), `None` seçilmedi |
| `target` | Yalnız bir stream seçildiyse: URL AIOStreams'in kendi host'unda mı (`same_host: true`, proxy/playback yolu) yoksa başka host'ta mı |

## URL çağırma (range okuma)

`SUBSYNC_RANGE_PROBE=1` ile açılır, varsayılan kapalı. Yalnız bir stream seçildiyse
ve URL HTTP ise, eşleşen URL'den **tek istek** ile `Range: bytes=0-65535`
istenir. Yönlendirmeler elle izlenir (en çok 5). Sunucu Range'i yok sayıp
bütün dosyayı gönderse bile okuma 64 KB'ta kesilir ve bağlantı kapatılır.
Log'a `k2 probe` satırının sonunda `range: Some(Ok(RangeReport { … }))` eklenir:

| Alan | Anlamı |
|---|---|
| `hops` | İstenen her URL'nin host türü (`AioStreams` / `RealDebrid` / `TorBox` / `Other`); ilki stream URL'sinin kendisi |
| `status` | Son yanıtın durum kodu |
| `partial` | 206 + `Content-Range`: Range'e uyuldu (K3'ün ilk sorusu) |
| `accept_ranges` | `Accept-Ranges: bytes` gönderildi |
| `size_matches` | Sunucunun bildirdiği dosya boyutu oynatıcının `videoSize`'ıyla aynı mı |
| `content` | `Content-Type` türü (`Matroska` / `OctetStream` / `OtherVideo` / `Other` / `Missing`) |
| `bytes_read` / `ebml_magic` | Okunan bayt; ilk 4 bayt EBML başlığı mı (dosya MKV mi) |

Yerel deneme (2026-10-27, sahte AIOStreams + 1 MB sahte MKV, Range desteklemeyen
`python3 -m http.server`): `status: 200, partial: false, bytes_read: 65536,
ebml_magic: true, size_matches: Some(true)`. Bütçe sınırı ve MKV tanıma çalışıyor.

## Çalıştırma

Sunucu ve Funnel açılırken ortamda `SUBSYNC_AIOSTREAMS_URL` de verilir; sonra
masaüstünde ve TV'de birkaç film/bölüm açılır, `k2 probe` satırları aşağıya
eklenir. Tercihen hem Real-Debrid hem TorBox'tan gelen stream'ler denenir.

## Sonuçlar

| Tarih | İstemci | İçerik | Debrid | streams | both | filename_exact | size | target | elapsed_ms |
|---|---|---|---|---|---|---|---|---|---|
| 2026-10-27 | Stremio masaüstü, macOS | film | Real-Debrid | 38 | 1 | 1 | 1 | `Http { same_host: false }` | 1469 |
| 2026-10-27 | Stremio masaüstü, macOS | dizi bölümü | Real-Debrid | 20 | 1 | 1 | 1 | `Http { same_host: false }` | 376 |
| 2026-10-27 | Stremio masaüstü, macOS | dizi bölümü | Real-Debrid | 14 | 1 | 1 | 1 | `Http { same_host: false }` | 416 |
| 2026-10-27 | Stremio masaüstü, macOS | dizi bölümü (range açık) | Real-Debrid | 16 | 1 | 1 | 1 | `Http { same_host: false }` | 601 |
| 2026-10-27 | Stremio masaüstü, macOS | aynı bölüm, 2. istek | Real-Debrid | 16 | 1 | 1 | 1 | `Http { same_host: false }` | 760 |
| 2026-10-31 | Stremio (VPS üzerinden) | film | TorBox | 29 | 1 | 1 | 1 | `Http { same_host: false }` | 5838 |

Range okuma sonuçları:

| Örnek | hops | status | partial | accept_ranges | size_matches | content | bytes_read | ebml_magic | elapsed_ms |
|---|---|---|---|---|---|---|---|---|---|
| bölüm, 1. istek | `[Other, RealDebrid]` | 206 | ✓ | ✓ | ✓ | `Other` | 65536 | ✓ | 1246 |
| bölüm, 2. istek | `[Other, RealDebrid]` | 206 | ✓ | ✓ | ✓ | `Other` | 65536 | ✓ | 658 |
| TorBox film (VPS IP'si) | `[Other, TorBox]` | 206 | ✓ | ✓ | ✓ | `Matroska` | 65536 | ✓ | 540 |

İkinci tur (2026-10-27, range ve K3 açık): 16 istek daha (film ve dizi,
9–41 stream); **hepsinde `both: 1`**. Ayrıntılar K3 notunda.

Üçüncü tur (2026-10-27): 20 istek daha (2–38 stream), hepsinde `both: 1`.
Bir bölümde iki stream aynı boyuttaydı (`size: 2`); dosya adı ayırdı. Yani
boyut tek başına yetmiyor, ikisi birlikte gerekli. Bir filmde Real-Debrid'in
bildirdiği boyut `videoSize`'dan farklıydı (`size_matches: false`), ama dosya
MKV'ydi; boyut karşılaştırması tek başına "gerçek dosya değil" demek değil.

## Sonuç (Real-Debrid, masaüstü)

- **Eşleşme:** 5/5 örnekte `filename` + `videoSize` tek stream'i kesin seçti
  (14–38 stream arasında). Eşleşme sunucuda, oynatıcının gönderdiği alanlarla
  yapılıyor; K1'e göre Android TV de aynı alanları gönderiyor.
- **URL davranışı:** Stream URL'si AIOStreams manifest host'undan farklı bir
  host'ta; tek yönlendirmeyle Real-Debrid indirme host'una gidiyor. Orada
  Range'e uyuluyor (206), bildirilen boyut `videoSize` ile aynı, dosya MKV.
  İlk 64 KB 0,7–1,2 sn'de geldi.
- Debrid'de hazır olmayan dosyalar da eşleşiyor; URL'leri gerçek dosya yerine
  başka bir host'tan, boyutu tutmayan, MKV olmayan bir video veriyor
  (`hops: [Other, Other]`). Range okuması bunu `size_matches: false` ile ayırt
  ediyor.
- Debrid tarafında ne tetiklendiği (ör. Real-Debrid'in "Downloads" geçmişine
  kayıt düşmesi) buradan görülemiyor; oynatmanın kendisi de aynı yolu
  kullandığı için ek bir yan etki beklenmiyor.
- Range okumaları şimdilik oynatmayla **aynı IP'den** (ev) yapıldı. VPS'ten
  okuma (farklı IP) K3/SS-006'da ayrıca denenmeli.

## Notlar

- **İlk ölçüm (masaüstü, film):** 38 stream'in hepsi `filename` ve `videoSize`
  taşıyor. Dosya adı da boyut da tek başına yalnız bir stream'e uyuyor, ikisi
  birlikte de tam bir stream'e (`both: 1`). Bu örnekte eşleşme kesin.
- Eşleşen stream'in URL'si AIOStreams'in kendi host'unda değil
  (`same_host: false`). Ya doğrudan debrid linki ya da başka bir alan adındaki
  playback yolu; hangisi olduğu ve çağrılınca ne tetiklediği ayrı denemede
  (kullanıcı onayıyla) bakılacak. K3'teki Range okumaları bu URL'ye yapılacak.
- **Dizi bölümleri (2 örnek):** aynı tablo; her ikisinde de `both: 1`. Şu ana
  kadar 3/3 örnekte filename + boyut tek stream'i kesin seçiyor, `filename_loose`
  hiçbirinde ek aday getirmedi.
- 2026-10-27'deki örneklerin hepsi Real-Debrid. TorBox: aşağıda (SS-007).
- Stream listesi ilk istekte ~1,5 sn, sonrakilerde ~0,4 sn'de geldi; altyazı yanıtını bekletmediği için oynatmayı
  etkilemedi. Gerçek akışta bu süre altyazı gecikmesine eklenecek; cache gerekli.
- `content: Other`: Real-Debrid `Content-Type` olarak video/octet-stream dışı
  bir değer gönderiyor; MKV olduğu EBML başlığından anlaşılıyor, tür başlığına
  güvenilmemeli.
- Aynı oturumda iki `route=none token=bad status=404` isteği düştü. Funnel genel
  adres olduğu için tarayıcı/bot istekleri olabilir; add-on'u etkilemiyor.

## TorBox (SS-007, 2026-10-31)

AIOStreams'te Real-Debrid ve TorBox birlikte açık; okumalar Oracle
sunucusunun IP'sinden, oynatma evden. İlk tur: 1 film.

- **Eşleşme:** 29 stream'de `both: 1`. TorBox stream'leri de `filename` ve
  `videoSize` taşıyor; eşleşme Real-Debrid'deki gibi çalışıyor.
- **URL davranışı:** tek yönlendirmeyle TorBox CDN'ine gidiyor
  (`hops: [Other, TorBox]`). Range'e uyuluyor (206), boyut `videoSize` ile
  aynı. Real-Debrid'den farkı: `Content-Type` doğru (`Matroska`).
- **Süre:** stream listesi 5,8 sn. RD'li sunucu ölçümlerinde de ilk istek
  5,5–8,4 sn sürdüğü oldu; TorBox'a özgü sayılmaz. İlk 64 KB 0,54 sn.
- **Yan etki:** dosya TorBox'ta hazırdı; hazır olmayan bir dosyada URL
  çağrısının indirme başlatıp başlatmadığı bu turda görülmedi.
- **Seçim kuralı notu:** aynı dosya iki debrid'de de varsa listede ilk gelen
  okunur; okumanın hangi debrid'e gittiği `hops`'tan anlaşılır.

## Boyutla eşleşme (SS-020, 2026-10-29)

Kapsama sayımında 21 dosyanın 3'ü K2'de düştü; eşleşmeyen bölümde ad hiçbir
stream'de tutmuyor, boyut 2 stream'de tutuyordu. Seçim kuralı artık:

1. Ad ve boyut birlikte tutan ilk stream (`matched_by: Some(Both)`).
2. Yoksa boyutu tutan ilk stream (`Some(Size)`). Aynı bayt sayısı aynı dosya
   sayılır; birden çok kopyadan hangisinin okunduğu fark etmez.
3. Boyut hiç tutmuyorsa (ya da oynatıcı boyut göndermediyse) eşleşme yok.

Aynı ad + boyutla birden çok stream (`both: 2+`) eskiden belirsiz sayılıp
okunmuyordu; aynı kurala göre artık ilki okunur.
