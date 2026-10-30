# K5 — Stremio altyazı dosyasını yeniden çekiyor mu? (SS-042)

## Soru

Stremio altyazı listesini oynatma başında bir kez istiyor; liste o oynatma
içinde değişmiyor. Peki listedeki bir girdinin dosyasını
(`/<token>/sub/<girdi>/<n>.srt`) her seçimde yeniden çekiyor mu? Çekiyorsa
çeviri yedeği (SS-026) menüye baştan sabit bir girdi koyup, seçildiği anda o
ana kadar çevrilen kısmı döndürebilir; ikinci açılışı beklemek gerekmez.

## Yöntem

- Sunucu her dosya isteğinde `fetch` olayı yazar:
  `subtitle fetched entry=<girdi> n=<sıra> bytes=<boyut>` (metin yok).
- Dosya yanıtı `Cache-Control: no-store` ile gider; oynatıcının yanıtı
  saklamaması istenir.
- Deneme: oynatmada SubSync girdisini seç; kapatıp aynı girdiyi yeniden
  seç; başka bir girdiye geçip geri dön; videoyu ileri sar. Olaylar sunucu
  logundan (`journalctl -u stremio-subsync`) okundu.

## Apple TV 4K (2026-10-30, Oracle sunucusu)

Bir film; menüde 1 SubSync girdisi (diskten, 121 120 bayt).

| Saat (UTC) | Olay |
|---|---|
| 19:09:46 | liste isteği (`videoHash` yok) → diskten, 1 girdi |
| 19:09:48 | liste isteği (`videoHash` var) → bellekten, 1 girdi |
| 19:09:53 | dosya çekildi (ilk seçim) |
| 19:10:22 | dosya çekildi |
| 19:10:33 | dosya çekildi |
| 19:10:46 | dosya çekildi |

Dört adımda dört çekme; her biri 200, aynı boyut. Liste ikinci kez
istenmedi (oynatma başındaki iki istek SS-041'deki çift istek).

## Sonuç

- **Apple TV dosyayı her seçimde yeniden çekiyor.** İlk seçim dışında en az
  iki yeniden seçim (kapat → seç, başka girdi → geri dön) yeni bir istek
  üretti. İleri sarmanın da çekme yapıp yapmadığı bu kayıttan kesin
  ayrılmıyor (dördüncü çekme ona ya da fazladan bir seçime ait olabilir).
- SS-026 için yol açık: menüde sabit bir "çeviri" girdisi, seçimde o ana
  kadar çevrilen kısım; kullanıcı ileride kapatıp yeniden seçerek devamını
  alır.
- Ölçülmedi: dosya iki çekme arasında uzadığında oynatıcının yeni içeriği
  kullandığı (bu denemede içerik aynıydı), masaüstü ve Android TV. Bunlar
  SS-026'nın gerçek denemesinde görülür.
