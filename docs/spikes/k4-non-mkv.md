# K4 — MKV olmayan dosyalar (SS-032, SS-033)

## Soru

Debrid'den gelen MKV olmayan dosyalar hangi kapsayıcıda, içlerinde İngilizce
metin altyazı izi var mı? MP4'te varsa cetvel `moov`'dan ucuza çıkıyor mu?

## Yöntem

- İlk 64 KB'tan kapsayıcı: MP4 (`ftyp`/`moov`/`mdat`/`wide`/`free`), AVI
  (`RIFF…AVI `), MPEG-TS / M2TS (188 / 192 baytta bir 0x47), bilinmeyen.
- MP4'te üst seviye kutular yürünür, `mdat` atlanır, `moov` bütçe içinde
  (8 MiB, 6 istek) okunur. İz listesi: `hdlr` türü, `stsd` kodeği, `mdhd`
  dili, örnek sayısı, metin izlerinde dolu örnek sayısı; `mvex` → parçalı.
- İngilizce `tx3g`/`text` izinden olaylar: `stts` + `stsz` (≤ 2 bayt boş),
  `elst` kaydırması; 50 satırdan kısa izler cetvel olmaz.
- Panelde "MP4 ✓ / MP4 (parçalı) / AVI / TS" ve gömülü altyazılar satırı;
  log'da `k2 probe` satırında `container` ve `mp4` raporu (yalnız sayı ve
  kod; dosya adı, iz adı, metin yok).

## Ölçümler

Sunucu: proje kökünde `SUBSYNC_DUMP_DIR=dumps cargo run`; dosya Stremio'da
açılır, satırlar panelden okunur.

| # | Tarih | İçerik | Kapsayıcı | `moov` başta mı | İstek / bayt | Parçalı | Altyazı izleri (dil kodek: dolu/örnek) | Cetvel | Menü |
|---|---|---|---|---|---|---|---|---|---|
| 1 | | | | | | | | | |

## Sonuç

(ölçümden sonra; SS-034 kararına girdi)
