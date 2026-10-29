# İşler

Durum: `[ ]` sırada · `[~]` sürüyor · `[x]` bitti (kanıtıyla)

Güncel kod ve iş listesi `main` üzerindedir. Her SS numarası tek bir işi
tanımlar; yeni numara bu dosyadan ayrılır. Eski dal numaralarının eşlemesi:
[`docs/history/branch-consolidation.md`](docs/history/branch-consolidation.md).
Sonraki boş numara: **SS-029**.

## M0 — İskelet

- [x] **SS-001** Token'lı manifest + subtitles route'ları, `extra` ayrıştırma — kanıt: `cargo test` (10 test, yanlış token → 404 negatif testi dahil)
- [x] **SS-002** Masaüstü Stremio'ya `localhost` linkiyle kurulum denemesi — kanıt: Stremio 5.1.30'dan istek logu düştü (`docs/spikes/k1-playback-hints.md`)

## M1 — Keşif

- [~] **SS-003** K1: Masaüstü, Android TV ve Apple TV'nin gönderdiği alanları ölç (istek logu yalnız var/yok basıyor) — kanıt: `docs/spikes/k1-playback-hints.md` (masaüstü ve Android TV bitti; Apple TV bekliyor, Tailscale Funnel ile ölçülecek)
- [x] **SS-004** K2: AIOStreams stream listesinde filename + boyutla eşleşme ve URL davranışı — kanıt: `docs/spikes/k2-aiostreams-match.md` (Real-Debrid, masaüstü: 5/5 `both: 1`; URL tek yönlendirmeyle RD'ye gidiyor, 206 Range). TorBox → SS-007
- [x] **SS-005** K3: Real-Debrid ve TorBox linklerinde Range desteği; 10+ gerçek MKV'de altyazı izinin Cues'ta indekslenme oranı — kanıt: `docs/spikes/k3-mkv-cues.md` (RD: Range var; İngilizce metin izi 12 dosyanın 11'inde indekslenmiş). TorBox → SS-007, VPS IP → SS-008
- [ ] **SS-006** VPS + HTTPS kurulumu (Caddy + systemd). Script'ler hazır ama denenmedi; Oracle ARM kapasitesi dolu olduğu için ertelendi — kanıt: `https://<alan adı>/<token>/manifest.json` çalışıyor, yanlış token 404
- [~] **SS-020** K2: dosya adı tutmayıp boyutu tutan stream'i eşleşme say. Kapsama sayımında 21 dosyanın 3'ü (%14) burada düştü; eşleşmeyen bölümde ad 0, boyut 2 stream'de tutuyordu (aynı bayt sayısı ≈ aynı dosya). Boyut birden çok stream'de tutuyorsa hepsi aynı boyutta olduğundan ilki alınır; boyut hiç tutmuyorsa eşleşme yok — kanıt: testler + eşleşmeyen bölümde cetvel çıkması. **Öncelik 1** (küçük, düşük risk). Kod ve testler hazır (`streams::match_streams`, log'da `matched_by: Some(Size)`); daha önce eşleşmeyen bölümü Stremio'da yeniden açıp cetvelin çıktığını görmek kalıyor. O bölüm için "dosya bulunamadı" cevabı cache'lendiyse 1 gün geçerli; menüde hâlâ o görünürse süre dolmasını beklemek ya da `cache/`'i temizlemek gerekir
- [ ] **SS-007** TorBox ile K2 eşleşme ve K3 Range ölçümlerini tekrarla (abonelik yenilenince) — kanıt: K2/K3 notlarında TorBox satırları
- [ ] **SS-008** VPS IP'sinden Real-Debrid Range okuması (oynatma ev IP'sinden sürerken) — kanıt: K3 notunda VPS satırı

> Real-Debrid keşfi M2–M5 için yeterliydi. Apple TV, TorBox ve VPS ölçümleri
> açık; bunlar yerelde tamamlanan işleri geri açmaz.

## M2 — Cetvel

- [x] **SS-009** Gerçek MKV'lerde altyazı cue noktalarında `CueDuration` ve `CueRelativePosition` var mı ölç (cetvel yalnız Cues'tan çıkabilir mi) — kanıt: `docs/spikes/m2-ruler.md` (5/5 dosyada her cue noktası süreli → evet)
- [x] **SS-010** Cetvel: İngilizce metin izinin olay zamanlarını (başlangıç + süre) Cues'tan çıkar; metin okunmaz — kanıt: testler + `docs/spikes/m2-ruler.md` (1006–2356 olay; 44 KB–1.28 MB, 1–2 istek, ≤1.4 sn)

## M3 — Kaynaklar

Hedef dil `SUBSYNC_LANG` (ISO 639-2, varsayılan `tur`). Testler fake ile;
gerçek servis normal kullanımda çağrılır (M5); testler sağlayıcı ağı kullanmaz.

- [x] **SS-015** SRT okuma: bayt → metin (UTF-8 / UTF-16 / dile göre eski kod sayfası), satır zamanları; bir `Fix` ile yeniden yazma — kanıt: `cargo test srt` (7 test; CP1254, bozuk bloklar, gidiş-dönüş)
- [x] **SS-016** OpenSubtitles v3 (anahtarsız Stremio add-on'u): IMDb id + dil için adaylar, indirme — kanıt: testler + `docs/spikes/m3-sources.md` (4 içerikte 2–7 Türkçe aday, 12 indirmenin hepsi okundu)
- [~] **SS-017** SubDL (ücretsiz anahtar, `SUBDL_API_KEY`): adaylar, ZIP içinden doğru SRT — kanıt: testler (gerçek deneme anahtar alınınca). Kapsama sayımında Türkçe aday 38 aramanın 35'inde bulundu; bu yüzden öncelik düşük, ama "uyan yok" durumlarında ikinci kaynak işe yarar
- [ ] **SS-019** İki parçalı (CD1/CD2) altyazıları birleştir: aynı sürümün parçaları art arda eklenip tek aday olarak hizalanır — kanıt: testler + gerçek bir CD1/CD2 örneği
- [x] **SS-018** Adayları dosya adına benzerliğe göre sırala (sürüm grubu, kaynak, çözünürlük; CAM/TS en sona) — kanıt: `cargo test rank` + gerçek denemede aynı sürüm 165 puanla başta

## M4 — Hizalama

Girdi iki zaman listesi: cetvel olayları (M2) ve aday altyazının satırları.
Çekirdek teslim akışına bağlıdır. Gerçek eşik kalibrasyonu SS-014
kapsamında sürer.

- [x] **SS-011** Hizalama çekirdeği: kendimiz mi, `alass-core` mu? Küçük karşılaştırma — kanıt: `docs/spikes/m4-align.md` (220 sentetik hizalamada doğruluk aynı, bizimki ~27× hızlı ve kabul testli → kendimiz) + `docs/decisions.md` satırı
- [x] **SS-012** Kaydırma + fps düzeltmesi: aday satırlarını cetvele en iyi oturtan (ölçek, kaydırma) — kanıt: `cargo test align` (örtüşme eğrisi kaba kuvvetle birebir; etiketli sette fps hep doğru, kaydırma ≤100 ms)
- [x] **SS-013** Kabul testi (baş / orta / son aynı kaydırmada tutmalı) + sentetik etiketli test seti; yanlış kabul yok — kanıt: `labeled_set_has_no_false_accepts` (8 tohum × 14 durum; 56/56 pozitif kabul, 56/56 negatif ret; çeviri ve CD1 durumları dahil)
- [~] **SS-014** Gerçek etiketli set: M2 cetvelleri + M3 adaylarıyla eşikleri ayarla (gerçek dosyalar elle etiketlenir) — kanıt: `docs/spikes/m4-align.md` tablosu
- [x] **SS-021** Yerel senkron: altyazıyı tek kaydırma yerine yerel olarak kaydır — her satıra, çevresindeki ~20 satırın İngilizce iz başlangıçlarına göre ortanca kaydırması (satır satır yapıştırma eşleşme hatasında satırı saniyelerce kaçırır; 6 sabit nokta gürültüyü kovalar). Etiketleme modunda ayrı menü girdisi olarak denenir. Karar kaydındaki "bölmeli hizalama M6'da" maddesini açar — kanıt: ölçüm notu (…5984'te satırların %36'sı 300 ms, %74'ü 700 ms içinde eşleşiyor) + kullanıcı etiketleri + sentetik negatifleri yine reddeden testler → **sonuç: fayda görülmedi** (kullanıcı fark görmedi; kayan altyazıda ortanca 165 ms, yön de cetvelin kendi erkenliği yüzünden ters). Teslimde yok, yalnız etiketleme modunda.

## M5 — Teslim

- [x] **SS-022** Kabul edilen altyazıları Stremio'da normal sun: istek probe'ları bekler (en çok 15 sn), yalnız kabul edilenler `tur` dilinde, fix uygulanmış olarak listelenir; aynı çevirinin kopyaları tek girdi — kanıt: testler + 2026-10-28 Stremio'da kullanıcı "Türkçe altyazı geldi ve senkrondu" (bölüm: 3 kabulden kopyalar teke indi, 1 sunuldu; film: 2 sunuldu)
- [x] **SS-023** Sonucu cache'le: bellek + disk (`SUBSYNC_CACHE_DIR`, varsayılan `cache`); dosya adı istek anahtarının özeti, kimlik/dosya adı yok; kabul 30 gün, "uygun aday yok" 1 gün geçerli — kanıt: testler + gerçek oturumda tekrar açılışlar `served from=memory`, `cache/`'te 2 dosya (diskten okuma yalnız testte denendi)
- [~] **SS-024** Sonuç çıkmazsa boş bekletme: menüde nedeni gösteren tek girdi ("SubSync: İngilizce iz yok", "dosya bulunamadı", "aday altyazı yok", "uyan altyazı yok", "MKV değil", "geçici hata", "hazırlanıyor"); seçilince açıklama altyazı olarak görünür. Cetvel yoksa Türkçe aramayı beklemeden hemen cevap. Kalıcı nedenler 1 gün (disk dahil), geçiciler 60 sn yalnız bellekte; 15 sn aşılırsa "hazırlanıyor" — kanıt: testler + Stremio'da cetvelsiz bir dosyada nedenin görünmesi
- [~] **SS-025** PGS (resim) İngilizce izden cetvel. Kapsama sayımında 21 dosyanın 6'sı (%29) yalnız PGS İngilizce içeriyordu; en büyük kazanç. Cues'ta PGS izlerinin süresiz cue noktaları var; hem SRT hem PGS İngilizce içeren iki dosyada PGS noktası SRT satırının ~2 katı (2008 → 4020, 1061 → 2406) → her satır "göster" + "temizle" çifti. Adımlar: (1) PGS cue zamanlarını dump'a ekle, (2) SRT + PGS içeren bir dosyada PGS çiftlerinden çıkan satırları aynı dosyanın SRT'siyle hizala (beklenen: fix 1/1, ~0 ms, yüksek isabet), (3) tutarsa PGS'i cetvel olarak kullan (metin izi yoksa) — kanıt: ölçüm notu + testler + PGS'li bir dosyada kabul edilen altyazı. **Öncelik 2** (kullanıcının SRT + PGS içeren bir dosyayı bir kez açması gerekir). Adım 1–2 kodda: `SUBSYNC_DUMP_DIR` açıkken İngilizce PGS cue zamanları `dumps/pgs/` altına yazılıyor (metin cetveli olmasa da), `cargo run -- pgs-check` iki fazı ve her-nokta varyantını aynı dosyanın metin cetvelleriyle karşılaştırıyor; testler 105 (sentetikte `Even` fazı 1/1, +0 ms, isabet 1.00). Gerçek ölçüm ve adım 3 bekliyor — `docs/spikes/m5-pgs-ruler.md`

## Çeviri yedeği (M6'dan öne çekildi)

> M5 bir istek için kabul edilen aday sunamıyorsa, gömülü İngilizce metin izi
> hedef dile çevrilmesi planlanır; henüz uygulanmadı. Karar:
> `docs/decisions.md` (çeviri yedeği). Önce SS-028 ile blokların gerçekten
> okunabildiği ve maliyeti ölçülür. PGS cetveli (SS-025) ayrı bir iştir.

- [~] **SS-028** Spike: İngilizce metin izinin blok metnini Cues konumlarından (`CueClusterPosition` + `CueRelativePosition`) hedefli oku; satır başına Range ile çoklu aralık (`bytes=a-b,c-d,…`) Real-Debrid'de karşılaştırılır. Metin loglanmaz, yalnız sayılar — kanıt: `docs/spikes/m6-block-text.md` (`with_relative`, istek sayısı, bayt, süre, okunan/beklenen satır; 5+ gerçek dosya)
- [ ] **SS-026** Çeviri yedeği: Nen'in çeviri çekirdeğini taşı (blok + bağlam penceresi, doğrulama/onarım, iki aşamalı belge analizi, OpenRouter adapter'ı); zamanlar modele gitmez, cetvelden birleştirilir; "otomatik çeviri" etiketli SRT, M5 cache'iyle; ilk istek işi arka planda başlatır — kanıt: sahte çeviriciyle testler (sağlayıcı ağı yok) + gerçek bir bölümde Stremio'da görünen çeviri

## Bakım

- [x] **SS-027** Dallar `main` üzerinde birleştirildi; M5 menü/hata önbelleği ve blok probe'u birlikte korundu. Paralel M2/M3 ölçümleri aktarıldı, numaralar tekilleştirildi, eski devam notları düzeltildi. Altı eski dal `codex/archive/` altında; açık Claude süreçleri kullandığı için worktree dizinleri korundu. 7 cache + 14 dump dosyası yedeklendi — kanıt: `docs/history/branch-consolidation.md`, 95 geçen test, temiz format/Clippy ve tüm arşiv uçlarının `main` atası olması.

## Devam notları (2026-10-27, dallar birleştirildi)

- **Yerel M5 çalışıyor.** Dosya eşleştirme → İngilizce metin cetveli → aday
  indirme → kaydırma/FPS → kabul testi → Stremio'ya SRT + cache bağlıdır.
  Altyazı üretilemezse kısa neden gösterilir. SS-024'ün kodu ve testleri
  hazır; gerçek Stremio denemesi henüz kaydedilmediği için iş açık.
- **Kapsama darboğazı cetvel:** 21 farklı dosyanın 10'unda İngilizce metin
  izi, 6'sında yalnız PGS var; 3'ünde AIOStreams eşleşmedi, 2'sinde İngilizce
  yok/okunamıyor. Türkçe aday 38 aramanın 35'inde bulundu. Kaynak ve örneklem
  sınırları: `docs/spikes/m4-align.md`, “Kapsama”.
- **Öncelikler:** SS-020 dosya eşleşmesi → SS-025 PGS cetvel ölçümü → SS-006
  VPS/HTTPS. SS-014 gerçek etiketleri artırma, SS-017 SubDL gerçek denemesi,
  SS-019 CD1/CD2 birleştirme de açık. Bu oturum yalnız dalları toparladı;
  bu özellikler uygulanmadı.
- **Çeviri yolu:** SS-028'in kodu `src/blocks.rs`'de hazır; 5+ gerçek SRT/ASS
  izli dosyada maliyet ölçümü bekleniyor (`docs/spikes/m6-block-text.md`).
  Sonuca göre SS-026 çeviri yedeği uygulanır. SS-025 PGS yalnız zaman cetveli
  üretmeyi hedefler; metin çevirisi sağlamaz.
- **M2 konum ölçümü:** ana ölçüm tablosu `with_relative` sayısını kaydetmedi;
  paralel M2 kaydı 5/5 dosyada bulunduğunu bildiriyor. İki kayıt da
  `docs/spikes/m2-ruler.md` içinde korunur; aynı dosyalar olabilir. Bu kayıt,
  gerçek blok okuma başarısı ve maliyeti için SS-028'in yerini tutmaz.
- **SS-014:** 4 kullanıcı etiketi: 3 doğru kabul, 1 yanlış ret, 0 yanlış
  kabul. Yerel senkron (SS-021) fayda göstermedi; yalnız etiketleme modunda.
  `cargo run -- calibrate dumps` yerel zaman kayıtlarını yeniden değerlendirir.
- **Sunucu:** proje kökünde, ortam değişkenlerinin hazır olduğu terminalde
  `SUBSYNC_DUMP_DIR=dumps cargo run`. `SUBSYNC_TOKEN` (en az 32 karakter) ve
  `SUBSYNC_AIOSTREAMS_URL` gerekir. Range ve kaynaklar varsayılan açık,
  `SUBSYNC_LANG=tur`; SubDL yalnız `SUBDL_API_KEY` varsa açılır.
- **Blok ölçümü:** aynı komuta `SUBSYNC_BLOCK_PROBE=1` ekle. Probe arka planda
  çalışır. Cache'ten yanıtlanan dosyada probe çalışmaz; ölçüm için daha önce
  açılmamış dosyalar seç. Çıktı `m6 blocks`, yalnız sayı ve tür taşır.
- **Yerel veriler:** 7 cache ve 14 dump dosyası hem köke hem
  `.local/branch-backups/` altına kopyalanıp doğrulandı. Eski dallar
  `codex/archive/` altında; worktree'ler açık Claude süreçleri kullandığı
  için silinmedi. Güncel geliştirme kökteki `main` üzerinde yapılır.
  Ayrıntı birleştirme kaydında.
- **Geçici yayın:** Tailscale Funnel (443 → `127.0.0.1:7000`). Kapatmak:
  `tailscale funnel --https=443 off`. Kalıcı VPS kurulumu henüz yok.
- **Açık keşif:** SS-003 Apple TV, SS-007 TorBox (abonelik bekliyor; URL
  çağrısı indirme başlatabilir, ilk gerçek deneme kullanıcı onayıyla),
  SS-008 VPS IP'sinden RD okuması.
- **Güvenlik notu:** önceki oturumda açığa çıkan SubSync token'ı VPS'e geçerken
  yenilenmeli; AIOStreams şifresi keşif ölçümleri bitince değiştirilmeli.
  Linkler sohbete veya loga yazılmaz.
