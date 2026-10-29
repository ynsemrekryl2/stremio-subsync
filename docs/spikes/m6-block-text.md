# M6 — Satır metni Cues konumlarından okunabilir mi?

Soru (SS-028): Çeviri yedeği için cetvel izinin her satırının metni gerekir.
Cues her satırın bloğunu gösteriyorsa (`CueClusterPosition` +
`CueRelativePosition`), dosyanın geri kalanı okunmadan satır başına küçük bir
Range isteğiyle metin alınabilir mi? Kaç istek, kaç bayt, ne kadar sürer?
Real-Debrid tek istekte birden fazla aralık veriyor mu?

## Yöntem

`SUBSYNC_BLOCK_PROBE=1` açıkken, K2 probe'u cetvel izini seçtikten sonra arka
planda (teslimi geciktirmeden) şunu yapar:

- Cetvel izinin cue noktalarından blok konumları çıkar: küme konumu (Segment
  verisine göre) + küme içi konum. İkisi birden olmayan nokta atlanır.
- Önce bir kez, ilk 3 bloğun 64 baytı **tek istekte** istenir
  (`Range: bytes=a-b,c-d,e-f`); yalnız yanıtın türü kaydedilir, gövde okunmaz.
- Sonra her blok için bir Range isteği (dosya başına en çok 4000 blok).
  İlk ölçümde 16'sı aynı anda gidiyordu; 2026-10-29'dan beri 4'ü aynı anda,
  reddedilen ya da düşen okuma (bağlantı, zaman aşımı, 429/5xx) 0.5 / 1.5 /
  4 sn bekleyip yeniden deneniyor: kümenin boyut alanı 1–8 bayt olabildiği için 7 + 1024 baytlık
  pencere okunur, içinde izin numarası tutan `BlockGroup`/`SimpleBlock`
  aranır. Blok pencereden büyükse ikinci bir okuma yapılır.
- Metin tutulmaz: yalnız yük boyutu ve geçerli UTF-8 olup olmadığı sayılır.
  İz `ContentEncodings` taşıyorsa (`encoded: true`) yük sıkıştırılmış olabilir;
  bu durumda UTF-8 sayısı düşer.

Log satırı: `m6 blocks kind=… report=BlockReport { … }`.

| Alan | Anlamı |
|---|---|
| `cue_refs` / `with_cluster` / `with_relative` | İzin cue noktaları; küme ve küme içi konum taşıyanlar |
| `targets` / `attempted` | Farklı blok konumu; okunmaya çalışılan |
| `found` / `missing` / `failed_reads` | Blok bulundu / okuma çalıştı ama blok yok / okuma hatası |
| `failures` | Okuma hatalarının nedeni: `timeout`, `connect`, `status` ((kod, sayı) çiftleri), `range_ignored`, `other`, `task` |
| `utf8` / `empty` / `laced` / `extended` | Geçerli UTF-8 yük / boş yük / lacing'li blok / ikinci okuma gerekti |
| `payload_bytes` | Toplam yük (metin) boyutu |
| `retries` | `requests` içinde bekleyip yeniden denenen istekler |
| `requests` / `bytes_read` / `elapsed_ms` | Blok okumalarının maliyeti (çoklu aralık denemesi hariç) |
| `multi_range` | `Multipart` (çoklu aralık çalışıyor) / `SingleOnly` / `Ignored` (200) / `Status(…)` / `Failed` |

Yerel deneme: birim testleri (1, 4 ve 8 baytlık küme boyut alanı, `BlockGroup`
ve `SimpleBlock`, başka izin bloğu, pencereden büyük blok, sıkıştırılmış ve
lacing'li yük, dosya sonunu aşan konum). Gerçek dosya henüz yok.

## Sonuçlar

| # | İçerik | İz (codec) | cue_refs | with_relative | found / attempted | utf8 | encoded | istek | bayt | süre | multi_range |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | dizi bölümü | SRT | 576 | 576 | 395 / 576 | 395 | hayır | 576 | 407 245 | 53.6 sn | SingleOnly |
| 2 | dizi bölümü | SRT | 546 | 546 | 328 / 546 | 328 | hayır | 546 | 338 168 | 56.3 sn | SingleOnly |
| 3 | dizi bölümü | SRT | 527 | 527 | 399 / 527 | 399 | hayır | 527 | 411 369 | 21.8 sn | SingleOnly |
| 4 | dizi bölümü | SRT | 551 | 551 | 295 / 551 | 295 | hayır | 551 | 304 145 | 29.4 sn | SingleOnly |
| 5 | dizi bölümü | SRT | 569 | 569 | 267 / 569 | 267 | hayır | 569 | 275 277 | 13.4 sn | SingleOnly |
| 6 | film | SRT | 1835 | 1835 | 990 / 1835 | 990 | hayır | 1835 | 1 020 690 | 202.0 sn | SingleOnly |
| 7 | film | SRT | 2022 | 2022 | 818 / 2022 | 818 | hayır | 2022 | 843 358 | 134.0 sn | SingleOnly |
| 8 | film | SRT | 2137 | 2137 | 592 / 2137 | 592 | hayır | 2137 | 610 352 | 73.2 sn | SingleOnly |
| 9 | bölüm (4 eşzamanlı + tekrar deneme) | ASS | 6622 | 6622 | **3842 / 4000** | 3842 | hayır | 4600 (600 tekrar) | 3 961 102 | 734.9 sn | SingleOnly |

2026-10-29, Real-Debrid, masaüstü Stremio, ev bağlantısı. `missing`,
`extended`, `laced` ve `empty` her dosyada 0.

## Notlar

- **Konumlar doğru:** başarılı her okumada blok tam Cues'un gösterdiği yerde
  bulundu (`missing: 0`) ve yükün tamamı geçerli UTF-8 (1684 / 1684). Satır
  başına ortalama ~32 bayt metin; hiçbir blok 1 KB pencereyi aşmadı.
- **Çoklu aralık yok:** Real-Debrid tek istekte birden fazla aralığı
  vermiyor (`SingleOnly`), yani satır başına bir istek gerekiyor: bölüm
  başına ~550 istek, ~0.3–0.4 MB.
- **Okumaların %39'u başarısız** (2769 okumanın 1085'i), süre 13–56 sn
  (16 eşzamanlı istek). `missing` 0 olduğu için sorun konumda değil, istekte;
  büyük olasılıkla sunucu tarafı sınırlama. 1–5'te neden kaydedilmedi;
  rapora `failures` eklendi.
- **Neden: bağlantı reddi.** Filmlerde (6–7) başarısızlıkların %97'si
  `connect` (818 / 845 ve 1172 / 1204), kalanı zaman aşımı; HTTP hata kodu
  hiç yok. Sunucu yeni bağlantıları kabul etmiyor: 16 eşzamanlı istek
  Real-Debrid'in dosya ya da IP başına bağlantı sınırını aşıyor olabilir.
  Filmde süre 2–3.5 dakika.
- **Düzeltmeden sonra (9):** 4 eşzamanlı istek ve bekleyip tekrar deneme
  ile okumaların **%96'sı** başarılı (öncesi %28–61). İlk denemede reddedilen
  istek yine %13 (4600 isteğin 600'ü tekrar); 158 blok üç tekrardan sonra da
  `connect` ile kaldı. Bedeli hız: saniyede ~5.4 blok. Bu bölümün en kalabalık
  izi tabela/şarkı dahil 6622 parçalı bir ASS izi; yalnız ilk 4000 blok
  okundu, 12 dakika sürdü. Diyalog izi seçilseydi (~335 satır) ~1 dakika.
- **Tahmini maliyet (düzeltmeden sonra):** ~550 satırlık bölüm ~1.5–2 dk,
  ~2000 satırlık film ~6–7 dk, satır başına ~1 KB okuma.
- **Çeviri yedeği için ara sonuç:** metin Cues konumlarından okunabiliyor;
  düşük eşzamanlılık ve tekrar denemeyle okuma %96'ya çıktı. SS-026 için
  açık kalanlar: (1) çeviri için metin izi diyalog izi olmalı, tabela/şarkı
  izi değil; (2) kalan ~%4 blok için ikinci tur ya da satırı boş bırakma;
  (3) filmde dakikalar sürdüğü için ilk istekte arka planda başlatma
  (kararda zaten öyle).
