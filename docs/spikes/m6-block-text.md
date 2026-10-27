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
- Sonra her blok için bir Range isteği (16'sı aynı anda, dosya başına en çok
  4000 blok): kümenin boyut alanı 1–8 bayt olabildiği için 7 + 1024 baytlık
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
| `utf8` / `empty` / `laced` / `extended` | Geçerli UTF-8 yük / boş yük / lacing'li blok / ikinci okuma gerekti |
| `payload_bytes` | Toplam yük (metin) boyutu |
| `requests` / `bytes_read` / `elapsed_ms` | Blok okumalarının maliyeti (çoklu aralık denemesi hariç) |
| `multi_range` | `Multipart` (çoklu aralık çalışıyor) / `SingleOnly` / `Ignored` (200) / `Status(…)` / `Failed` |

Yerel deneme: birim testleri (1, 4 ve 8 baytlık küme boyut alanı, `BlockGroup`
ve `SimpleBlock`, başka izin bloğu, pencereden büyük blok, sıkıştırılmış ve
lacing'li yük, dosya sonunu aşan konum). Gerçek dosya henüz yok.

## Sonuçlar

| # | İçerik | İz (codec) | cue_refs | with_relative | found / attempted | utf8 | encoded | istek | bayt | süre | multi_range |
|---|---|---|---|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | — | — | — | — | — |

## Notlar

- (ölçümden sonra doldurulacak)
