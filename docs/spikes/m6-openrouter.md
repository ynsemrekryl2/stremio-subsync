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
