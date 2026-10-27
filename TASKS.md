# İşler

Durum: `[ ]` sırada · `[~]` sürüyor · `[x]` bitti (kanıtıyla)

## M0 — İskelet

- [x] **SS-001** Token'lı manifest + subtitles route'ları, `extra` ayrıştırma — kanıt: `cargo test` (10 test, yanlış token → 404 negatif testi dahil)
- [x] **SS-002** Masaüstü Stremio'ya `localhost` linkiyle kurulum denemesi — kanıt: Stremio 5.1.30'dan istek logu düştü (`docs/spikes/k1-playback-hints.md`)

## M1 — Keşif

- [~] **SS-003** K1: Masaüstü, Android TV ve Apple TV'nin gönderdiği alanları ölç (istek logu yalnız var/yok basıyor) — kanıt: `docs/spikes/k1-playback-hints.md` (masaüstü ve Android TV bitti; Apple TV bekliyor, Tailscale Funnel ile ölçülecek)
- [x] **SS-004** K2: AIOStreams stream listesinde filename + boyutla eşleşme ve URL davranışı — kanıt: `docs/spikes/k2-aiostreams-match.md` (Real-Debrid, masaüstü: 5/5 `both: 1`; URL tek yönlendirmeyle RD'ye gidiyor, 206 Range). TorBox → SS-007
- [x] **SS-005** K3: Real-Debrid ve TorBox linklerinde Range desteği; 10+ gerçek MKV'de altyazı izinin Cues'ta indekslenme oranı — kanıt: `docs/spikes/k3-mkv-cues.md` (RD: Range var; İngilizce metin izi 12 dosyanın 11'inde indekslenmiş). TorBox → SS-007, VPS IP → SS-008
- [ ] **SS-006** VPS + HTTPS kurulumu (Caddy + systemd). Script'ler hazır ama denenmedi; Oracle ARM kapasitesi dolu olduğu için ertelendi — kanıt: `https://<alan adı>/<token>/manifest.json` çalışıyor, yanlış token 404
- [ ] **SS-007** TorBox ile K2 eşleşme ve K3 Range ölçümlerini tekrarla (abonelik yenilenince) — kanıt: K2/K3 notlarında TorBox satırları
- [ ] **SS-008** VPS IP'sinden Real-Debrid Range okuması (oynatma ev IP'sinden sürerken) — kanıt: K3 notunda VPS satırı

> M1 sonucu planı değiştirebilir. M2+ işleri keşif bittikten sonra açılır.

## M2 — Cetvel

- [~] **SS-009** Gerçek MKV'lerde altyazı cue noktalarında `CueDuration` ve `CueRelativePosition` var mı ölç (cetvel yalnız Cues'tan çıkabilir mi) — kanıt: `docs/spikes/m2-ruler.md`
- [~] **SS-010** Cetvel: İngilizce metin izinin olay zamanlarını (başlangıç + süre) Cues'tan çıkar; metin okunmaz — kanıt: testler + gerçek dosyada olay sayısı ve okuma maliyeti (`docs/spikes/m2-ruler.md`)

## Devam notları (2026-10-27)

Yeni oturum buradan başlar; ayrıntılar ilgili dokümanlarda.

- **Geçici yayın:** Sunucu Mac'te `cargo run` ile, dışarıya **Tailscale Funnel**
  (443 → `127.0.0.1:7000`) ile açılıyor. Add-on masaüstü Stremio'ya Funnel
  adresiyle kurulu, hesap üzerinden TV'lere geçiyor. Token yalnız açık terminal
  sekmesinin ortamında; sekme kapanırsa yeni token üretilir ve add-on yeniden
  kurulur. Funnel'ı kapatmak: `tailscale funnel --https=443 off`.
- **SS-003:** Kalan tek ölçüm Apple TV (Stremio Lite). Sunucu + Funnel açıkken
  film aç, terminaldeki `subtitles request` satırını K1 notuna ekle.
- **SS-004 (K2):** Bitti (Real-Debrid). Sunucu `SUBSYNC_AIOSTREAMS_URL` ile
  başlatılınca her altyazı isteğinde stream listesi okunur; ayrıca
  `SUBSYNC_RANGE_PROBE=1` verilirse eşleşen URL'den ilk 64 KB okunur. Loglar
  yalnız sayı ve tür taşır. TorBox'ta URL çağırma indirme başlatabilir;
  TorBox gelince (SS-007) ilk denemeyi kullanıcı onayıyla yap.
- **SS-005 (K3):** Bitti (RD). `SUBSYNC_RANGE_PROBE=1` ile her eşleşen MKV'de
  Tracks ve Cues okunur. Real-Debrid dosya sonunu aşan Range'de bağlantıyı
  erken kesiyor; okumalar toplam boyutla sınırlanıyor.
- **SS-008 (K3) riski:** Range okumaları VPS'in IP'sinden, oynatma evin
  IP'sinden yapılacak. Real-Debrid aynı linkin farklı IP'lerden kullanılmasını
  ya da veri merkezi IP'lerini kısıtlayabilir; K3'te ayrıca bakılmalı.
- **SS-006:** Oracle Always Free ARM'de kapasite yoktu. Seçenekler:
  Pay-As-You-Go'ya yükseltmek (ücretsiz sınırda para çıkmaz) ya da Hetzner
  CAX11 (~4 €/ay). Script'ler `deploy/`, adımlar `docs/deploy.md`.
