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

## M4 — Hizalama

Girdi iki zaman listesi: cetvel olayları (M2) ve aday altyazının satırları.
M2 ölçümü ve M3 beklenmeden saf mantık olarak yazılır.

- [x] **SS-011** Hizalama çekirdeği: kendimiz mi, `alass-core` mu? Küçük karşılaştırma — kanıt: `docs/spikes/m4-align.md` (220 sentetik hizalamada doğruluk aynı, bizimki ~27× hızlı ve kabul testli → kendimiz) + `docs/decisions.md` satırı
- [x] **SS-012** Kaydırma + fps düzeltmesi: aday satırlarını cetvele en iyi oturtan (ölçek, kaydırma) — kanıt: `cargo test align` (örtüşme eğrisi kaba kuvvetle birebir; etiketli sette fps hep doğru, kaydırma ≤100 ms)
- [x] **SS-013** Kabul testi (baş / orta / son aynı kaydırmada tutmalı) + sentetik etiketli test seti; yanlış kabul yok — kanıt: `labeled_set_has_no_false_accepts` (8 tohum × 11 durum; 40/40 pozitif kabul, 48/48 negatif ret)
- [ ] **SS-014** Gerçek etiketli set: M2 cetvelleri + M3 adaylarıyla eşikleri ayarla (gerçek dosyalar elle etiketlenir) — kanıt: `docs/spikes/m4-align.md` tablosu

## Devam notları (2026-10-27, akşam)

Yeni oturum buradan başlar; ayrıntılar ilgili dokümanlarda.

- **Şu an sırada: SS-009 / SS-010 ölçümü.** Kod hazır (`src/ruler.rs`,
  `src/mkv.rs`). Kullanıcı sunucuyu `SUBSYNC_RANGE_PROBE=1 cargo run` ile
  başlatıp İngilizce SRT/ASS izli 5–6 dosya açacak; `k2 probe` satırlarındaki
  `cue_durations` ve `ruler: Some(RulerReport { … })` değerleri
  `docs/spikes/m2-ruler.md` tablosuna işlenecek. `with_duration` ≈ `cue_refs`
  ise cetvel ek okuma olmadan Cues'tan çıkıyor (SS-009 evet). Değilse süreler
  bloklardan okunmalı; plan değişir, önce ölçüm notuna yaz.
- **M4 çekirdeği hazır** (`src/align.rs`, sentetik set `src/synth.rs`).
  Henüz route'a bağlı değil (M5). Eşikler sentetik; gerçek ayar SS-014'te,
  M2 ölçümü ve M3 adayları gelince.
- **Sunucuyu başlatma (aynı terminal sekmesinde ortam hazırsa):**
  `SUBSYNC_RANGE_PROBE=1 cargo run`. Ortam kaybolduysa (yeni sekme) tek komut:
  `read` ile AIOStreams linkini sorar, yeni token üretir, kurulum linkini
  panoya kopyalar, sunucuyu başlatır; sonra masaüstü Stremio'da eski SubSync
  kaldırılıp panodaki link kurulur. Linkler sohbete yapıştırılmamalı.
- **Geçici yayın:** Mac'te `cargo run`, dışarıya **Tailscale Funnel**
  (443 → `127.0.0.1:7000`). Funnel'ı kapatmak: `tailscale funnel --https=443 off`.
- **Log satırları:** `http … route=… token=ok|bad` her istek; `subtitles request`
  ipuçları; `k2 probe … report / range / mkv` K2-K3-M2 ölçümü. Hepsi yalnız
  sayı ve tür taşır; URL, token, dosya adı, metin yok.
- **Bilinenler:** K2 (RD) eşleşme kesin; URL tek yönlendirmeyle RD'ye gidiyor,
  206 Range. K3 (RD) 12 MKV'nin 11'inde İngilizce metin izi Cues'ta. RD dosya
  sonunu aşan Range'de bağlantıyı erken kesiyor; okumalar toplam boyutla
  sınırlı. Debrid'de hazır olmayan dosya yer tutucu video döndürüyor
  (`hops`'ta RD yok, `ebml_magic: false`).
- **Açık M1 işleri:** SS-003 Apple TV · SS-006 VPS (Oracle ARM kapasitesi
  yoktu; PAYG ya da Hetzner CAX11) · SS-007 TorBox (abonelik bitti; URL
  çağırma indirme başlatabilir, ilk deneme kullanıcı onayıyla) · SS-008 VPS
  IP'sinden RD okuması (kullanıcı M2'den sonraya bıraktı).
- **Güvenlik:** SubSync token'ı ve AIOStreams manifest linki önceki sohbette
  açığa çıktı. VPS'e geçince yeni token; AIOStreams şifresi K-ölçümleri
  bitince değiştirilmeli.
