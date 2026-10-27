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
| 2026-10-27 | İlk sunucu **Oracle Cloud Always Free** ARM, HTTPS için **Caddy**, uygulama systemd servisi | Maliyet sıfır. Caddy sertifikayı kendisi alır. Erişim logu kapalı, çünkü URI token'ı taşır. Oracle boştaki sunucuyu geri alırsa Pay-As-You-Go'ya ya da Hetzner'e (~4 €/ay) geçilir. |
| 2026-10-27 | K2/K3 ölçümleri şimdilik **yalnız Real-Debrid** ile yapılır | TorBox aboneliği bitti. Probe debrid'den bağımsız; TorBox gelince aynı adımlar tekrarlanır (SS-007). TorBox'a özgü davranış (URL çağrısının indirme başlatması) o zamana kadar varsayım olarak kalır. |
| 2026-10-27 | Cetvel, **Cues'taki altyazı konumlarından** hedefli okunur; altyazısı Cues'ta olmayan dosyalar şimdilik desteklenmez | K3: Real-Debrid'deki 12 MKV'nin 11'inde İngilizce metin izi indekslenmiş; Cues 18–855 KB, 1–2 istek. İndekslenmemiş dosya için küme taraması pahalı; gerekirse M6'da yeniden düşünülür. |
| 2026-10-27 | **Hizalama çekirdeği bizim** (`src/align.rs`); `alass-core` kullanılmaz | 220 sentetik hizalamada doğruluk aynı (ortanca 3–4 ms, fps hep doğru), bizimki ~27 kat hızlı ve kabul testini içeriyor; `alass-core` ret kararı vermez ve GPL-3.0. Farklı kurgu dosyaları reddedilir; bölmeli hizalama gerekirse M6'da düşünülür. Ölçüm: `docs/spikes/m4-align.md`. |

## Açık

- (yok)
