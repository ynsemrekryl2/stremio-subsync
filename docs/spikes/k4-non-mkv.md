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

## ffmpeg ile üretilen dosyalar (2026-10-30)

Gerçek bir İngilizce `tx3g` MP4 bulunamadığı için okuyucu, gerçek bir
muxer'ın (ffmpeg 9.0.1, `mov_text`) çıktısında denendi. Ağ yok; yerel dosya
`cargo run -- mp4-check <dosya> [referans.srt]` ile sunucunun okuduğu gibi
okunur, ilk cetvel referansın zamanlarıyla karşılaştırılır.

Kaynak: 8 dakikalık 64×64 test videosu + sinüs sesi + 120 satırlık İngilizce
(ve bir varyantta Türkçe) SRT; süreler 0,9–4,2 sn, aralar 0,15–3 sn, ms'ler
düzensiz. Referans iki türlü: muxlanan SRT ve ffmpeg'in dosyadan geri
çıkardığı SRT (oynatıcının göreceği zamanlar).

| Dosya | `moov` | İstek / bayt | Bulunan izler | Cetvel | Fark (başlangıç / bitiş) |
|---|---|---|---|---|---|
| varsayılan (`moov` sonda) | `mdat`'tan sonra | 1 / 119 KB | avc1, mp4a, eng tx3g 120/241, tur tx3g 120/241 | eng, 120 satır | 0 / 0 ms |
| `+faststart` | başta (64 KB'a sığmıyor) | 1 / 57 KB | aynı | eng, 120 satır | 0 / 0 ms |
| `-itsoffset 2.5` | sonda | 1 / 112 KB | avc1, mp4a, eng tx3g | eng, 120 satır | muxlanan SRT'ye +2500 ms; çıkarılan SRT'ye 0 / 0 ms |
| QuickTime `.mov` | sonda | 1 / 112 KB | `text` handler + `text` kodek, dil `mac0` | eng, 120 satır | 0 / 0 ms |
| parçalı (`frag_keyframe+empty_moov`) | başta | 0 / 0 | izler var, örnek 0 | yok → "Biçim dışı" | — |
| altyazısız | sonda | 1 / 106 KB | avc1, mp4a | yok → "İng. iz yok" | — |

Notlar:
- ffmpeg kaydırmayı edit list ile değil, öndeki uzun bir boş örnekle yazıyor;
  `elst` (sürüm 1, 64 bit) okunuyor, metin izinde `media_time` 0. Sıfırdan
  farklı `media_time` ve boş edit yalnız birim testte deneniyor.
- Video ve ses izlerinin örnek tabloları da `moov` içinde olduğu için `moov`
  boyutu film süresiyle büyür; 8 dakikada ~120 KB. 2 saatlik bir filmde
  birkaç MB beklenir, bütçe (8 MiB) içinde.

## Ölçümler (gerçek dosyalar)

Sunucu: proje kökünde `SUBSYNC_DUMP_DIR=dumps cargo run`; dosya Stremio'da
açılır, satırlar panelden okunur.

| # | Tarih | İçerik | Kapsayıcı | `moov` başta mı | İstek / bayt | Parçalı | Altyazı izleri (dil kodek: dolu/örnek) | Cetvel | Menü |
|---|---|---|---|---|---|---|---|---|---|
| 1 | | | | | | | | | |

## Sonuç

(ölçümden sonra; SS-034 kararına girdi)
