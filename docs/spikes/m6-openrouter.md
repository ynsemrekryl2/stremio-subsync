# M6 — OpenRouter ile gerçek çeviri (SS-038)

## Soru

Nen'den taşınan çeviri çekirdeği (SS-037) ve OpenRouter adapter'ı gerçek
bir filmde baştan sona çalışıyor mu; ne kadar sürüyor, ne tutuyor? SS-026
için asıl soru: ilk satırlar ne zaman hazır, çeviri oynatmaya yetişiyor mu?

## Yöntem

`scripts/dev.sh translate-check <ingilizce.srt> <cikti.srt>`: yerel bir
İngilizce SRT, `openai/gpt-6-luna`, hedef `tur`. Ekrana yalnız sayılar
basılır (satır, blok, çağrı süresi, token, maliyet); metin yalnız çıktı
dosyasına gider. Bloklar sırayla çevrilir (40 satır, 6 satır bağlam).
Çıktı, kaynağın zamanlarıyla metne bakmadan karşılaştırıldı.

## Sonuçlar (2026-10-30)

| Deneme | Satır | Blok | Sonuç | Toplam | İlk blok | Çağrı | Giriş / önbellek / çıkış token | Maliyet |
|---|---|---|---|---|---|---|---|---|
| Nen kabul dosyası | 24 | 1 | ok | 24,0 sn | 24,0 sn | 2 | 1 886 / 0 / 2 215 | 0,0013 $ |
| Film, 1. deneme | 2 034 | 60 | 25. blokta düştü | 474 sn | 33,4 sn | 26 | 110 666 / 0 / 35 221 | 0,0314 $ |
| Film, 2. deneme | 2 034 | 60 | ok | 802 sn | 36,2 sn | 61 | 220 667 / 41 370 / 75 582 | **0,0606 $** |

Film yaklaşık 1 sa 50 dk; altyazı bir yorum (commentary) izi, İngilizce
metin olarak diyalogla aynı işi görüyor.

- **Analiz:** belgenin tamamı 41 373 giriş token'ı, 21–26 sn. İkinci
  denemede OpenRouter girdinin neredeyse tamamını önbellekten verdi.
- **Bloklar (2. deneme, 60 çağrı):** en kısa 5,6 sn, ortanca 11,1 sn,
  p90 16,7 sn, en uzun 69 sn. Onarım hiç gerekmedi (61 çağrı = 1 analiz +
  60 blok), yeniden deneme 0.
- **Çıktı:** 2 034 / 2 034 satır, zamanların hepsi kaynakla birebir aynı.
- **Hız:** saniyede ~2,5 satır çevriliyor; film saniyede ~0,3 satır
  ilerliyor, yani çeviri oynatmanın ~8 katı hızlı. İlk 37 satır (filmin
  ilk ~2 dk'sı) ~35 sn'de hazır.
- Çıkış token'ının girişe yakın olması modelin düşünme token'larından;
  bu fiyatta maliyete etkisi küçük.

## Paralel bloklar (SS-045, 2026-10-31)

Aynı film, aynı model; analizden sonra en çok 6 blok aynı anda
(`translate::PARALLEL_BLOCKS`), sonuçlar blok sırasıyla birleşiyor.

| Deneme | Sonuç | Toplam | İlk blok | Çağrı | Tekrar | Giriş / önbellek / çıkış token | Maliyet |
|---|---|---|---|---|---|---|---|
| Sıralı (yukarıdaki 2. deneme) | ok | 802 sn | 36,2 sn | 61 | 0 | 220 667 / 41 370 / 75 582 | 0,0606 $ |
| 6 paralel | ok | **153 sn** | 30,5 sn | 61 | 0 | 201 827 / 0 / 78 000 | 0,0642 $ |

- **~5,2 kat hızlı.** Analiz 22,8 sn, ardından 60 blok ~130 sn'de bitti.
- **Blok süresi değişmedi:** en kısa 5,6 sn, ortanca 11,4 sn, p90 14,5 sn,
  en uzun 17,7 sn (sıralıda ortanca 11,1 sn). 6 eşzamanlı istekte
  sağlayıcı yavaşlatmadı; 429 ve yeniden deneme 0, onarım 0.
- **İlk satırlar yine analizden sonra hazır** (30,5 sn); fark, sonrasının
  ~8 kat değil ~40 kat oynatmadan hızlı ilerlemesi: film saniyede ~13
  satır çevriliyor. ~1 dk'da ilk ~600 satır (filmin ~1/3'ü), 2,5 dk'da
  tamamı hazır.
- Maliyet aynı düzeyde; bu denemede önbellek hiç kullanılmadı (0,004 $ fark).
- Çıktı: 2 034 / 2 034 satır, zamanların hepsi kaynakla birebir aynı.
- Sonuç: 6 kalır. Darboğaz artık analiz (toplamın ~%15'i) ve blok
  dalgaları; 6'yı artırmak en çok ~1 dk kazandırır.

## Düşünme düzeyi (SS-046, 2026-10-31)

GPT-6 Luna OpenRouter'da varsayılan olarak `medium` düşünüyor
(`supported_efforts`: none, low, medium, high, xhigh, max).
`SUBSYNC_TRANSLATE_REASONING` ile aynı film, 6 paralel blok, sırayla
çalıştırıldı. `medium` satırı SS-045 denemesi (ayar verilmemiş; düşünme
token'ı o zaman ayrıca sayılmıyordu).

| Düzey | Toplam | İlk blok | Analiz | Blok ortanca / p90 / en uzun | Çıkış token | Düşünme token | Kayma (`balance`) | Maliyet |
|---|---|---|---|---|---|---|---|---|
| `medium` (varsayılan) | 153,1 sn | 30,5 sn | 22,8 sn | 11,4 / 14,5 / 17,7 sn | 78 000 | ölçülmedi (~31 000*) | 8 / 619 (%1,3) | 0,0642 $ |
| `low` | **90,6 sn** | **18,8 sn** | 12,4 sn | 7,2 / 7,9 / 9,7 sn | 47 339 | 296 (2 blok) | 5 / 619 (%0,8) | 0,0484 $ |
| `none` | 93,9 sn | 20,9 sn | 14,2 sn | 7,2 / 8,6 / 10,2 sn | 47 160 | 0 | 4 / 619 (%0,6) | 0,0489 $ |

\* `medium`'un fazladan çıkış token'ı, diğer ikisiyle aradaki fark.

- Üç denemede de 61 çağrı, tekrar ve onarım 0; 2 034 / 2 034 satır.
- `medium`'da blok başına ~1 300 çıkış token'ının ~550'si düşünme.
  Düşünme kapanınca blok ~11 sn'den ~7 sn'ye, analiz ~23 sn'den ~13
  sn'ye iniyor: film **~1,7 kat hızlı**, ilk satırlar ~10 sn erken,
  maliyet ~%25 düşük.
- `low` neredeyse hiç düşünmüyor (60 bloğun 2'sinde 113 ve 183 token);
  hızda `none` ile aynı. Model gerekirse düşünebildiği için güvenlik payı
  var.
- `balance` kayması üçünde de çok düşük; düşünmeyi kısmak satırlar arası
  içerik kaymasını artırmadı. Bu yalnız kayma ölçüsü, çeviri kalitesi
  değil.
- Kalite: üç çıktıdan rastgele 40 satır körleme yan yana konup kullanıcıya
  okutuluyor (sayfa yerel; metin depoya ve log'a girmez). Varsayılan bu
  okumayla seçilir.

## Gemini 3.8 Flash ile karşılaştırma (yarıda bırakıldı)

Aynı film `google/gemini-3.8-flash` ile (milyon token 0,825 $ giriş /
4,125 $ çıkış; GPT-6 Luna'nın ~8 katı) denendi. 1. çalıştırma 22. blokta
okunamayan bir cevapla düştü (o sırada bu durum onarıma girmiyordu,
nedeni loglanmıyordu; ikisi de düzeltildi). 2. çalıştırma 2 blok sonra
kullanıcı isteğiyle durduruldu: tam film ~1 $ tutacaktı.

| Blok başına (1. çalıştırma, 22 blok) | Gemini 3.8 Flash | GPT-6 Luna |
|---|---|---|
| Süre | ortanca 24,6 sn | ortanca 11,1 sn |
| Çıkış token'ı | ~3 660 | ~1 220 |
| Saniyede çıkış token'ı | ~143 | ~94 |
| Maliyet | ~0,017 $ | ~0,001 $ |

- Gemini daha hızlı yazıyor ama blok başına ~3 kat fazla token üretiyor
  (çoğu düşünme), bu yüzden blok iki kat geç bitiyor ve ~16 kat pahalı.
- Analiz Gemini'de daha kısa sürdü (17 sn), ilk blok iki modelde de ~36 sn.
- Kalite karşılaştırılamadı: hiçbir Gemini çalıştırması çıktı dosyası
  bırakmadı. Toplam harcama ~0,45 $.
- Sonuç: varsayılan GPT-6 Luna kalır.

## 1. denemedeki hata

25. blokta OpenRouter 200 başlığını hemen gönderdi, gövde 120 sn'lik
süre sınırına kadar gelmedi. Gövde okuma hatası yeniden deneme bütçesine
girmediği için bütün iş düştü. Düzeltme: başarılı başlıktan sonra kesilen
gövde de bağlantı hatası sayılır ve yeniden denenir
(`a_body_cut_short_after_200_is_retried`). 2. denemede bu durum çıkmadı;
en uzun blok 69 sn sürdü.

## SS-026 için anlamı

- Altyazı listesi oynatma başında bir kez isteniyor, dosya ise her
  seçimde yeniden çekiliyor (SS-042). Çeviri oynatmadan hızlı olduğu için
  oynatma sırasında bir iki yeniden seçim filmin tamamını kapsar: ~2. dk'da
  seçilirse ilk ~15 dk, ~15. dk'da seçilirse film çoğunlukla hazır.
- Öneri (karar SS-026'da): çevrilmemiş satırlar dosyada İngilizce kalsın
  (boşluk olmaz), çevrilen kısmın sonuna "çeviri sürüyor, yeniden seçin"
  satırı konsun; dizide sıradaki bölüm önceden çevrilsin.
- Ölçülmedi: ileri sarmanın dosyayı yeniden çektirip çektirmediği ve
  uzayan dosyanın Apple TV'de görünmesi.

## Satıra bölünmüş cümlede kayma (SS-044)

Soru: İngilizcede iki satıra bölünen cümle Türkçeye (fiil sonda)
çevrilince içerik bir satıra yığılıyor mu? Fikir
[better-ai-srt-translation](https://github.com/yusufipk/better-ai-srt-translation)
README'sinden. `balance-check <ingilizce.srt> <ceviri.srt>`: kaynakta cümle
bitmeden sonraki satıra geçen çiftlerde, ilk satırın harf payının
kaynaktan çeviriye ne kadar oynadığı (`moved`); 0.35'ten fazlası "kaymış".
Metin basılmaz.

**Öncesi (2026-10-31, yukarıdaki 2. denemenin çıktısı, prompt sürümü 2):**

| Eşik | Kaymış çift (619'dan) | İleri | Geri |
|---|---|---|---|
| > 0.15 | 105 | 35 | 70 |
| > 0.20 | 54 | 18 | 36 |
| > 0.25 | 23 | 6 | 17 |
| > 0.30 | 7 | 2 | 5 |
| **> 0.35** | **3 (%0,5)** | 1 | 2 |

`|moved|` ortanca 0.07, p90 0.19, p99 0.31. 2 034 satırın 619 çifti
bölünmüş cümle.

- README'deki "cümle ilk satıra yığılır, ikincisi boş kalır" durumu
  GPT-6 Luna'da bu filmde neredeyse yok (619'da 3). Nen'in istemi zaten
  her cue'yu ayrı id ile istiyor ve bağlam veriyor.
- Küçük kaymalar çoğunlukla geri yönde (ikinci satır daha fazla metin
  alıyor): Türkçede fiilin sona gelmesiyle uyumlu, doğal.

**Sonrası (2026-10-31, prompt sürümü 3, aynı film ve model):**

| Eşik | Sürüm 2 | Sürüm 3 |
|---|---|---|
| > 0.15 | 105 | 101 |
| > 0.20 | 54 | 57 |
| > 0.25 | 23 | 33 |
| > 0.30 | 7 | 14 |
| **> 0.35** | **3** (1 ileri, 2 geri) | **7** (3 ileri, 4 geri) |
| `\|moved\|` ortanca / p90 / p99 | 0.070 / 0.191 / 0.308 | 0.061 / 0.196 / 0.359 |
| Neredeyse boş satır (kaynak ≥10 harf, çeviri <%30) | 5 | 8 |
| Toplam harf oranı (çeviri / kaynak) | 1.017 | 1.011 |

Çalıştırma: `result=ok`, 955 sn, ilk blok 31 sn, 62 çağrı (1 analiz,
60 blok, 1 onarım), tekrar 0, 217 126 giriş / 0 önbellek / 97 342 çıkış
token, **0,0758 $**. Blok ortanca 15,0 sn (sürüm 2'de 11,1 sn), blok
başına ~1 560 çıkış token'ı (~1 220).

- **Kural fayda göstermedi.** Kaymış çift 3'ten 7'ye çıktı, kuyruk
  biraz kalınlaştı; ortanca hafifçe düştü. İki çalıştırmanın satırlarının
  yalnız 542 / 2 034'ü birebir aynı, yani tek çalıştırmalık fark
  gürültü içinde: kural ne iyileştirdi ne açıkça bozdu.
- Bedeli var: blok başına daha çok çıkış token'ı (büyük olasılıkla
  düşünme), ~%35 daha uzun blok süresi, ~%25 daha pahalı.
- Sonuç: sorun bu kurulumda zaten nadir (%0,5–1,1); kural kaldırıldı
  (prompt sürümü yine 2), `balance` ölçümü kalır (model ya da istem
  değişince tekrar bakmak için).
