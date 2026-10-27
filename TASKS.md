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
- [ ] **SS-020** K2: bir dizi bölümünde AIOStreams listesindeki 12 stream'in hiçbiri dosya adıyla eşleşmedi (boyut 2'sinde tuttu); nedenini bul — kanıt: K2 notunda açıklama
- [ ] **SS-007** TorBox ile K2 eşleşme ve K3 Range ölçümlerini tekrarla (abonelik yenilenince) — kanıt: K2/K3 notlarında TorBox satırları
- [ ] **SS-008** VPS IP'sinden Real-Debrid Range okuması (oynatma ev IP'sinden sürerken) — kanıt: K3 notunda VPS satırı

> M1 sonucu planı değiştirebilir. M2+ işleri keşif bittikten sonra açılır.

## M2 — Cetvel

- [x] **SS-009** Gerçek MKV'lerde altyazı cue noktalarında `CueDuration` ve `CueRelativePosition` var mı ölç (cetvel yalnız Cues'tan çıkabilir mi) — kanıt: `docs/spikes/m2-ruler.md` (5/5 dosyada her cue noktası süreli → evet)
- [x] **SS-010** Cetvel: İngilizce metin izinin olay zamanlarını (başlangıç + süre) Cues'tan çıkar; metin okunmaz — kanıt: testler + `docs/spikes/m2-ruler.md` (1006–2356 olay; 44 KB–1.28 MB, 1–2 istek, ≤1.4 sn)

## M3 — Kaynaklar

Hedef dil `SUBSYNC_LANG` (ISO 639-2, varsayılan `tur`). Testler fake ile;
gerçek servis yalnız elle açılan probe'da çağrılır.

- [x] **SS-015** SRT okuma: bayt → metin (UTF-8 / UTF-16 / dile göre eski kod sayfası), satır zamanları; bir `Fix` ile yeniden yazma — kanıt: `cargo test srt` (7 test; CP1254, bozuk bloklar, gidiş-dönüş)
- [x] **SS-016** OpenSubtitles v3 (anahtarsız Stremio add-on'u): IMDb id + dil için adaylar, indirme — kanıt: testler + `docs/spikes/m3-sources.md` (4 içerikte 2–7 Türkçe aday, 12 indirmenin hepsi okundu)
- [~] **SS-017** SubDL (ücretsiz anahtar, `SUBDL_API_KEY`): adaylar, ZIP içinden doğru SRT — kanıt: testler (gerçek deneme anahtar alınınca)
- [ ] **SS-019** İki parçalı (CD1/CD2) altyazıları birleştir: aynı sürümün parçaları art arda eklenip tek aday olarak hizalanır — kanıt: testler + gerçek bir CD1/CD2 örneği
- [x] **SS-018** Adayları dosya adına benzerliğe göre sırala (sürüm grubu, kaynak, çözünürlük; CAM/TS en sona) — kanıt: `cargo test rank` + gerçek denemede aynı sürüm 165 puanla başta

## M4 — Hizalama

Girdi iki zaman listesi: cetvel olayları (M2) ve aday altyazının satırları.
M2 ölçümü ve M3 beklenmeden saf mantık olarak yazılır.

- [x] **SS-011** Hizalama çekirdeği: kendimiz mi, `alass-core` mu? Küçük karşılaştırma — kanıt: `docs/spikes/m4-align.md` (220 sentetik hizalamada doğruluk aynı, bizimki ~27× hızlı ve kabul testli → kendimiz) + `docs/decisions.md` satırı
- [x] **SS-012** Kaydırma + fps düzeltmesi: aday satırlarını cetvele en iyi oturtan (ölçek, kaydırma) — kanıt: `cargo test align` (örtüşme eğrisi kaba kuvvetle birebir; etiketli sette fps hep doğru, kaydırma ≤100 ms)
- [x] **SS-013** Kabul testi (baş / orta / son aynı kaydırmada tutmalı) + sentetik etiketli test seti; yanlış kabul yok — kanıt: `labeled_set_has_no_false_accepts` (8 tohum × 14 durum; 56/56 pozitif kabul, 56/56 negatif ret; çeviri ve CD1 durumları dahil)
- [~] **SS-014** Gerçek etiketli set: M2 cetvelleri + M3 adaylarıyla eşikleri ayarla (gerçek dosyalar elle etiketlenir) — kanıt: `docs/spikes/m4-align.md` tablosu
- [x] **SS-021** Yerel senkron: altyazıyı tek kaydırma yerine yerel olarak kaydır — her satıra, çevresindeki ~20 satırın İngilizce iz başlangıçlarına göre ortanca kaydırması (satır satır yapıştırma eşleşme hatasında satırı saniyelerce kaçırır; 6 sabit nokta gürültüyü kovalar). Etiketleme modunda ayrı menü girdisi olarak denenir. Karar kaydındaki "bölmeli hizalama M6'da" maddesini açar — kanıt: ölçüm notu (…5984'te satırların %36'sı 300 ms, %74'ü 700 ms içinde eşleşiyor) + kullanıcı etiketleri + sentetik negatifleri yine reddeden testler → **sonuç: fayda görülmedi** (kullanıcı fark görmedi; kayan altyazıda ortanca 165 ms, yön de cetvelin kendi erkenliği yüzünden ters). Teslimde yok, yalnız etiketleme modunda.

## M5 — Teslim

- [x] **SS-022** Kabul edilen altyazıları Stremio'da normal sun: istek probe'ları bekler (en çok 15 sn), yalnız kabul edilenler `tur` dilinde, fix uygulanmış olarak listelenir; aynı çevirinin kopyaları tek girdi — kanıt: testler + 2026-10-28 Stremio'da kullanıcı "Türkçe altyazı geldi ve senkrondu" (bölüm: 3 kabulden kopyalar teke indi, 1 sunuldu; film: 2 sunuldu)
- [x] **SS-023** Sonucu cache'le: bellek + disk (`SUBSYNC_CACHE_DIR`, varsayılan `cache`); dosya adı istek anahtarının özeti, kimlik/dosya adı yok; kabul 30 gün, "uygun aday yok" 1 gün geçerli — kanıt: testler + gerçek oturumda tekrar açılışlar `served from=memory`, `cache/`'te 2 dosya (diskten okuma yalnız testte denendi)
- [~] **SS-024** Sonuç çıkmazsa boş bekletme: menüde nedeni gösteren tek girdi ("SubSync: İngilizce iz yok", "dosya bulunamadı", "aday altyazı yok", "uyan altyazı yok", "MKV değil", "geçici hata", "hazırlanıyor"); seçilince açıklama altyazı olarak görünür. Cetvel yoksa Türkçe aramayı beklemeden hemen cevap. Kalıcı nedenler 1 gün (disk dahil), geçiciler 60 sn yalnız bellekte; 15 sn aşılırsa "hazırlanıyor" — kanıt: testler + Stremio'da cetvelsiz bir dosyada nedenin görünmesi
- [ ] **SS-025** PGS (resim) İngilizce izden cetvel: Cues'ta PGS izlerinin de cue noktası var (ör. 1506), süresiz; gösterme/temizleme olayları çiftlenerek satır zamanları çıkabilir mi ölç. Gerçek oturumda bir filmin tek İngilizce altyazısı PGS'ti — kanıt: ölçüm notu (PGS cue zamanları ile aynı yapımın SRT'si karşılaştırması)

## Devam notları (2026-10-28)

Yeni oturum buradan başlar; ayrıntılar ilgili dokümanlarda.

- **M5 teslim çalışıyor (yerel).** Stremio'da kabul edilen Türkçe altyazı
  senkron geldi, tekrar açılışlar cache'ten. Sunucu: Terminal sekmesinde
  `SUBSYNC_DUMP_DIR=dumps cargo run` (`SUBSYNC_TOKEN` ve
  `SUBSYNC_AIOSTREAMS_URL` o sekmede tanımlı). Sıradaki adaylar: SS-024
  (cetvelsiz istekleri cache'le), SS-020 (K2 eşleşmeyen bölüm), SS-006 (VPS,
  M5'in son parçası), SS-017 (SubDL anahtarı).
  Not: gerçek kullanımda dosyaların önemli kısmında İngilizce metin izi yok;
  bunlarda altyazı sunulamıyor (karar gereği).
- **SS-014 durumu:** 4 etiket: 3 doğru kabul, 1 yanlış ret, 0 yanlış kabul
  (`docs/spikes/m4-align.md`). `cargo run -- calibrate dumps` kayıtları
  yeniden hizalar.
- **M3 probe:** `SUBSYNC_SOURCES_PROBE=1` ile her istekte OpenSubtitles v3
  (+ `SUBDL_API_KEY` varsa SubDL) aranır, en iyi 3 aday indirilir;
  `m3 probe` satırı yalnız sayı/puan taşır. AIOStreams'ten bağımsız çalışır.
  SubDL anahtarı henüz yok (SS-017).
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
