# Kararlar

| Tarih | Karar | Gerekçe |
|---|---|---|
| 2026-10-27 | **Rust**, tek binary (axum + tokio) | Matroska okuma ve hizalama CPU-yoğun; Nen'deki altyazı kodu (GPL-3.0) gerekirse taşınabilir. |
| 2026-10-27 | **Küçük VPS**'te çalışır, HTTPS reverse proxy arkasında | Android TV ve Apple TV'den her zaman erişilebilir olmalı. |
| 2026-10-27 | **Tek kullanıcı.** Anahtarlar ortam değişkeninde; erişim yalnız kurulum linkindeki gizli token ile | Hesap sistemi, şifreli anahtar deposu ve ayar sayfası kişisel kullanımda gereksiz. Yanlış token → 404. |
| 2026-10-27 | **Dosya kimliği AIOStreams üzerinden** bulunur | Kullanıcı Real-Debrid ve TorBox'ı AIOStreams arkasında kullanıyor; iki debrid için ayrı API yazmak gerekmez. K2 ile doğrulanacak. |
| 2026-10-27 | **Cetvel = dosyanın gömülü İngilizce metin altyazısı**; ses kullanılmaz | Aynı dosyadan geldiği için en güvenilir referans; okuması ucuz. Gömülü İngilizce yoksa senkronlu altyazı üretilmez. |
| 2026-10-27 | Önce yalnız **MKV**, Cues üzerinden hedefli Range okuma | Debrid sürümlerinin büyük kısmı MKV. Tüm dosyayı taramak bütçe dışı. K3 ile doğrulanacak. |
| 2026-10-27 | Testi geçmeyen altyazı **sunulmaz** | Yanlış senkronlu altyazı, hiç altyazı olmamasından kötü. "Doğrulanmamış" iz ileride açıkça işaretli bir seçenek olabilir. |

## Açık

- Hizalama çekirdeği kendimiz mi yazılacak, yoksa `alass-core` (GPL-3.0) mı kullanılacak? M4 başında küçük bir karşılaştırmayla seçilecek.
