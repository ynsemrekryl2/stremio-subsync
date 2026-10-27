# İşler

Durum: `[ ]` sırada · `[~]` sürüyor · `[x]` bitti (kanıtıyla)

## M0 — İskelet

- [x] **SS-001** Token'lı manifest + subtitles route'ları, `extra` ayrıştırma — kanıt: `cargo test` (10 test, yanlış token → 404 negatif testi dahil)
- [x] **SS-002** Masaüstü Stremio'ya `localhost` linkiyle kurulum denemesi — kanıt: Stremio 5.1.30'dan istek logu düştü (`docs/spikes/k1-playback-hints.md`)

## M1 — Keşif

- [~] **SS-003** K1: Masaüstü, Android TV ve Apple TV'nin gönderdiği alanları ölç (istek logu yalnız var/yok basıyor) — kanıt: `docs/spikes/k1-playback-hints.md` (masaüstü ve Android TV bitti; Apple TV bekliyor, Tailscale Funnel ile ölçülecek)
- [~] **SS-004** K2: AIOStreams stream listesinde filename + boyutla eşleşme ve URL davranışı — kanıt: `docs/spikes/k2-aiostreams-match.md` (probe yazıldı; masaüstünde 3/3 örnek `both: 1`, Real-Debrid. URL çağırma denemesi ve TV örneği bekliyor)
- [ ] **SS-005** K3: Real-Debrid ve TorBox linklerinde Range desteği; 10+ gerçek MKV'de altyazı izinin Cues'ta indekslenme oranı — kanıt: `docs/spikes/k3-mkv-cues.md`
- [ ] **SS-006** VPS + HTTPS kurulumu (Caddy + systemd). Script'ler hazır ama denenmedi; Oracle ARM kapasitesi dolu olduğu için ertelendi — kanıt: `https://<alan adı>/<token>/manifest.json` çalışıyor, yanlış token 404
- [ ] **SS-007** TorBox ile K2 eşleşme ve K3 Range ölçümlerini tekrarla (abonelik yenilenince) — kanıt: K2/K3 notlarında TorBox satırları

> M1 sonucu planı değiştirebilir. M2+ işleri keşif bittikten sonra açılır.

## Devam notları (2026-10-27)

Yeni oturum buradan başlar; ayrıntılar ilgili dokümanlarda.

- **Geçici yayın:** Sunucu Mac'te `cargo run` ile, dışarıya **Tailscale Funnel**
  (443 → `127.0.0.1:7000`) ile açılıyor. Add-on masaüstü Stremio'ya Funnel
  adresiyle kurulu, hesap üzerinden TV'lere geçiyor. Token yalnız açık terminal
  sekmesinin ortamında; sekme kapanırsa yeni token üretilir ve add-on yeniden
  kurulur. Funnel'ı kapatmak: `tailscale funnel --https=443 off`.
- **SS-003:** Kalan tek ölçüm Apple TV (Stremio Lite). Sunucu + Funnel açıkken
  film aç, terminaldeki `subtitles request` satırını K1 notuna ekle.
- **SS-004 (K2):** Probe hazır. Sunucu `SUBSYNC_AIOSTREAMS_URL` ile
  başlatılınca her altyazı isteğinde stream listesi okunur ve `k2 probe`
  satırı yalnız sayıları loglar. Stream URL'lerine istek atılmaz, çünkü
  TorBox'ta indirme başlatabilir; URL çağırma ayrı ve kullanıcı onayıyla
  denenir. Ölçüm adımları `docs/spikes/k2-aiostreams-match.md`.
- **SS-005 (K3) riski:** Range okumaları VPS'in IP'sinden, oynatma evin
  IP'sinden yapılacak. Real-Debrid aynı linkin farklı IP'lerden kullanılmasını
  ya da veri merkezi IP'lerini kısıtlayabilir; K3'te ayrıca bakılmalı.
- **SS-006:** Oracle Always Free ARM'de kapasite yoktu. Seçenekler:
  Pay-As-You-Go'ya yükseltmek (ücretsiz sınırda para çıkmaz) ya da Hetzner
  CAX11 (~4 €/ay). Script'ler `deploy/`, adımlar `docs/deploy.md`.
