# İşler

Durum: `[ ]` sırada · `[~]` sürüyor · `[x]` bitti (kanıtıyla)

Güncel kod ve iş listesi `main` üzerindedir. Her SS numarası tek bir işi
tanımlar; yeni numara bu dosyadan ayrılır. Eski dal numaralarının eşlemesi:
[`docs/history/branch-consolidation.md`](docs/history/branch-consolidation.md).
Sonraki boş numara: **SS-043**.

## M0 — İskelet

- [x] **SS-001** Token'lı manifest + subtitles route'ları, `extra` ayrıştırma — kanıt: `cargo test` (10 test, yanlış token → 404 negatif testi dahil)
- [x] **SS-002** Masaüstü Stremio'ya `localhost` linkiyle kurulum denemesi — kanıt: Stremio 5.1.30'dan istek logu düştü (`docs/spikes/k1-playback-hints.md`)

## M1 — Keşif

- [x] **SS-003** K1: Masaüstü, Android TV ve Apple TV'nin gönderdiği alanları ölç (istek logu yalnız var/yok basıyor) — kanıt: `docs/spikes/k1-playback-hints.md` (üç istemcide de `filename`, `videoSize`, `videoHash` var; Apple TV 4K 2026-10-30 Tailscale Funnel üzerinden ölçüldü)
- [x] **SS-004** K2: AIOStreams stream listesinde filename + boyutla eşleşme ve URL davranışı — kanıt: `docs/spikes/k2-aiostreams-match.md` (Real-Debrid, masaüstü: 5/5 `both: 1`; URL tek yönlendirmeyle RD'ye gidiyor, 206 Range). TorBox → SS-007
- [x] **SS-005** K3: Real-Debrid ve TorBox linklerinde Range desteği; 10+ gerçek MKV'de altyazı izinin Cues'ta indekslenme oranı — kanıt: `docs/spikes/k3-mkv-cues.md` (RD: Range var; İngilizce metin izi 12 dosyanın 11'inde indekslenmiş). TorBox → SS-007, VPS IP → SS-008
- [x] **SS-006** VPS + HTTPS kurulumu (Caddy + systemd), Oracle ARM. **2026-10-30:** M0'dan kalan betikler bugünkü uygulamaya göre düzeltildi: AIOStreams linki ve SubDL anahtarı Mac'in Anahtar Zinciri'nden ssh stdin'iyle sunucuya gider (`deploy/env.sh`, ekrana ve komut satırına düşmez); cache `StateDirectory` altında (eskiden yazılabilir dizin yoktu); rsync `.gitignore`'a uyar (cache, dump ve worktree'ler sunucuya gitmez); `deploy/deploy.sh link` kurulum linkini panoya kopyalar. Sunucuda token yeniden üretilir (eski Funnel token'ı taşınmaz). — kanıt: `https://<alan adı>/<token>/manifest.json` çalışıyor, yanlış token 404 → **2026-10-30 doğrulandı:** Oracle ARM (eu-turin-1, `ssh oracle`) üzerine `deploy/deploy.sh` ile kuruldu, alan adı `92-4-220-81.sslip.io` (ücretsiz, satın alma yok). İlk derleme sunucuda 2 dk 21 sn. Oracle Security List'te yalnız 22 açıktı; 80/443 OCI CLI ile eklendi, ardından Caddy Let's Encrypt sertifikasını aldı. Dışarıdan: manifest 200 (geçerli sertifika, `resources: subtitles`), yanlış token manifest ve panel 404, panel 200; Caddy ve servis loglarında token 0 kez geçiyor. Kurulum linki `deploy/deploy.sh link` ile panoya. Aynı gün Stremio'ya bu linkle kuruldu; ilk gerçek oynatmada menüye 1 altyazı kondu (OpenSubtitles adayı, PGS cetvelinde %94 isabet) ve kullanıcı onu seçti (SS-008)
- [x] **SS-020** K2: dosya adı tutmayıp boyutu tutan stream'i eşleşme say. Kapsama sayımında 21 dosyanın 3'ü (%14) burada düştü; eşleşmeyen bölümde ad 0, boyut 2 stream'de tutuyordu (aynı bayt sayısı ≈ aynı dosya). Boyut birden çok stream'de tutuyorsa hepsi aynı boyutta olduğundan ilki alınır; boyut hiç tutmuyorsa eşleşme yok — kanıt: testler + eşleşmeyen bölümde cetvel çıkması. **Öncelik 1** (küçük, düşük risk). Kod ve testler hazır (`streams::match_streams`, log'da `matched_by: Some(Size)`); daha önce eşleşmeyen bölümü Stremio'da yeniden açıp cetvelin çıktığını görmek kalıyor. O bölüm için "dosya bulunamadı" cevabı cache'lendiyse 1 gün geçerli; menüde hâlâ o görünürse süre dolmasını beklemek ya da `cache/`'i temizlemek gerekir → **2026-10-29 doğrulandı:** 39 stream'li bir filmde ad tutmadı (`filename_exact: 0`), boyut tek stream'de tuttu (`matched_by: Some(Size)`); cetvel çıktı, 3 altyazı kabul edilip sunuldu, kullanıcı kayma görmedi
- [ ] **SS-007** TorBox ile K2 eşleşme ve K3 Range ölçümlerini tekrarla (abonelik yenilenince) — kanıt: K2/K3 notlarında TorBox satırları
- [x] **SS-008** VPS IP'sinden Real-Debrid Range okuması (oynatma ev IP'sinden sürerken) — kanıt: K3 notunda VPS satırı → **2026-10-30 doğrulandı:** sunucudaki ilk gerçek oynatmada dosya eşleşti, Tracks/Cues ve iki PGS cetveli okundu, oynatma kesilmedi

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
- [x] **SS-017** SubDL (ücretsiz anahtar, `SUBDL_API_KEY`; `scripts/dev.sh subdl` ile Anahtar Zinciri'nde): adaylar, ZIP içinden doğru SRT — kanıt: testler + **2026-10-30 gerçek deneme:** OpenSubtitles'ta 0 Türkçe adayı olan bir filmde SubDL 4 aday buldu, 3 ZIP'ten de SRT okundu, en-GB/en-US izlerine 0–3 ms ve %94–98 isabetle kabul edildi, menüde 2 girdi sunuldu ve Stremio altyazıyı çekti (`docs/spikes/m3-sources.md`)
- [ ] **SS-040** Aday genişliği: indirilen ilk 3 adayın hiçbiri kabul almazsa sıradakiler (en çok 3 daha) indirilip denenir — kanıt: testler + daha önce "uyan yok" alan bir dosyada sonraki adayın kabulü. **Ölçüm (2026-10-30):** 18 farklı dump'ın 15'inde SS-039 sonrası kabul var. Kabulsüz 3 dosyanın 2'sinde aramada toplam 2 aday çıkmış (denenecek başka aday yok); yalnız 1'inde (etiketli "kullanılabilir ama gevşek" bölüm) 3'ten fazla aday olmuş olabilir. Dump'lar aramada kaç aday bulunduğunu tutmuyor, bu yüzden kesin pay bilinmiyor. Beklenen kazanç küçük; az adaylı "uyan yok" durumlarında ikinci kaynak (SS-017 SubDL) daha umutlu
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
- [x] **SS-039** Basamaklı kaymada ikinci şans: global kabul testi `Disagree`/`LowHits` ile reddettiğinde, SS-021'in yerel kaydırmasıyla yeniden değerlendir; yerel kaydırma aralığı ≤1200 ms ve üç pencerenin hepsinde yerel isabet ≥%75 ise kabul et, `srt::render_local` ile sun. Ölçüm (2026-10-30, `align::local_check`, `calibrate` çıktısında `local=`): 27 gerçek dump'ta kural yalnız basamaklı kaymalı bölümün 3 adayını kabule çevirir; sentetik negatiflerde yerel isabet en çok %36, pozitiflerde en az %91. Kod ve testler hazır (`Verdict::accepted_locally`, teslimde `render_local`, global kabuller önce; `synth::stepped` ile yerel sunulanlarda ortanca hata ≤60 ms; etiketli sette yerel yanlış kabul de 0; `calibrate` 3 `ACCEPT-local`). **2026-10-30 doğrulandı:** aynı bölümde 3 Türkçe aday yerel geçişle kabul edildi (pencere isabeti %81–85, aralık 751–970 ms), menüde 3 `tur` girdisi sunuldu (önce "Uyan yok"); kullanıcı seçtiği girdiyi baş, orta (16–30. dk) ve sonda senkron buldu. İkinci gerçek kabul: yalnız PGS bir filmde (yavaş ~0.55 sn kayma, yerel aralık 1015 ms) kullanıcı baş/orta/son senkron buldu Bir bölümde 6 adayın 6'sı orta pencerede düşmüştü, yerel kaydırmayla isabet %23–62'den %83–86'ya çıkıyor (`docs/spikes/m4-align.md`, "Basamaklı kayma") — kanıt: sentetik etiketli sette yanlış kabul hâlâ 0 (basamaklı kayma pozitif durum olarak eklenir) + bu bölümün dump'ı `calibrate` ile kabul + Stremio'da baş/orta/son senkron

## M5 — Teslim

- [x] **SS-022** Kabul edilen altyazıları Stremio'da normal sun: istek probe'ları bekler (en çok 15 sn), yalnız kabul edilenler `tur` dilinde, fix uygulanmış olarak listelenir; aynı çevirinin kopyaları tek girdi — kanıt: testler + 2026-10-28 Stremio'da kullanıcı "Türkçe altyazı geldi ve senkrondu" (bölüm: 3 kabulden kopyalar teke indi, 1 sunuldu; film: 2 sunuldu)
- [x] **SS-023** Sonucu cache'le: bellek + disk (`SUBSYNC_CACHE_DIR`, varsayılan `cache`); dosya adı istek anahtarının özeti, kimlik/dosya adı yok; kabul 30 gün, "uygun aday yok" 1 gün geçerli — kanıt: testler + gerçek oturumda tekrar açılışlar `served from=memory`, `cache/`'te 2 dosya (2026-10-29 gerçek oturumda yeniden başlatma sonrası `served from=disk` görüldü)
- [x] **SS-024** Sonuç çıkmazsa boş bekletme: menüde nedeni gösteren tek girdi ("SubSync: İngilizce iz yok", "dosya bulunamadı", "aday altyazı yok", "uyan altyazı yok", "MKV değil", "geçici hata", "hazırlanıyor"); seçilince açıklama altyazı olarak görünür. Cetvel yoksa Türkçe aramayı beklemeden hemen cevap. Kalıcı nedenler 1 gün (disk dahil), geçiciler 60 sn yalnız bellekte; 15 sn aşılırsa "hazırlanıyor" — kanıt: testler + Stremio'da cetvelsiz bir dosyada nedenin görünmesi → **2026-10-29 doğrulandı:** yalnız PGS içeren bir filmde menüde "SubSync: İngilizce iz yok" girdisini kullanıcı gördü (`notice=Some(ImageOnly)`); başka bir bölümde 3 aday da ret alınca "uyan altyazı yok" sunuldu (`NoneAccepted`)
- [x] **SS-025** PGS (resim) İngilizce izden cetvel. Kapsama sayımında 21 dosyanın 6'sı (%29) yalnız PGS İngilizce içeriyordu; en büyük kazanç. Cues'ta PGS izlerinin süresiz cue noktaları var; hem SRT hem PGS İngilizce içeren iki dosyada PGS noktası SRT satırının ~2 katı (2008 → 4020, 1061 → 2406) → her satır "göster" + "temizle" çifti. Adımlar: (1) PGS cue zamanlarını dump'a ekle, (2) SRT + PGS içeren bir dosyada PGS çiftlerinden çıkan satırları aynı dosyanın SRT'siyle hizala (beklenen: fix 1/1, ~0 ms, yüksek isabet), (3) tutarsa PGS'i cetvel olarak kullan (metin izi yoksa) — kanıt: ölçüm notu + testler + PGS'li bir dosyada kabul edilen altyazı. **Öncelik 2** (kullanıcının SRT + PGS içeren bir dosyayı bir kez açması gerekir). Adım 1–2 kodda: `SUBSYNC_DUMP_DIR` açıkken İngilizce PGS cue zamanları `dumps/pgs/` altına yazılıyor (metin cetveli olmasa da), `cargo run -- pgs-check` iki fazı ve her-nokta varyantını aynı dosyanın metin cetvelleriyle karşılaştırıyor; testler 105 (sentetikte `Even` fazı 1/1, +0 ms, isabet 1.00). Adım 2 gerçek dosyada tuttu (2026-10-29, SRT + PGS içeren 2 bölüm): ilk İngilizce PGS izinin `Even` fazı iki metin cetveliyle de fix 1/1, +0 ms, isabet 1.00; doğru faz medyandan seçilebiliyor (Even 1.9–2.4 sn, Odd 160–292 ms); bazı ek İngilizce PGS izleri tutmuyor, kabul testi eliyor. Bir filmde de (SRT + PGS) `Even` ~0 ms ile kabul, `Each` ret → faz `Even`. Yalnız PGS içeren iki filmde faz ölçüldü (`Even` uzun satır %0, `Odd` %15–19). Adım 3 kodda: metin izi yoksa İngilizce PGS izleri `Even` fazıyla cetvel (`pgs::rulers`, 50 satırdan kısa izler atlanır); **2026-10-29 doğrulandı:** daha önce "İng. resim" alan yalnız PGS bir filmde Türkçe aday +19.2 sn kaydırmayla kabul edilip sunuldu, kullanıcı baş/orta/sonda senkron buldu — `docs/spikes/m5-pgs-ruler.md`

## MKV dışı dosyalar

> Şimdiye kadar EBML başlığıyla başlamayan her dosya "MKV değil" alıp
> bırakılıyordu. Önce ne olduklarını ölç; MP4'te İngilizce metin izi varsa
> cetvel `moov` örnek tablolarından çıkar (medya ve metin okunmaz). Gömülü
> İngilizcesi olmayanlar için yol ölçümden sonra seçilir.

- [~] **SS-032** K4: MKV olmayan dosyaları tanı — kapsayıcıyı ilk 64 KB'tan ayırt et (MP4 `ftyp`, AVI `RIFF…AVI `, MPEG-TS senkron baytları); MP4'te `moov`'u bulup izleri listele (tür, kodek, dil, örnek sayısı, dolu örnek sayısı, fragmented mı); panelde ve log'da görünür — kanıt: testler + `docs/spikes/k4-non-mkv.md` (kullanıcının MKV olmayan dosyası + karşılaşılan diğerleri)
- [~] **SS-033** MP4 metin izinden cetvel: İngilizce `tx3g`/`text` izinin dolu örneklerinden (`stts` + `stsz`, `elst` kaydırması) olay listesi, mevcut hizalama/teslim yoluna girer — kanıt: testler + (böyle bir dosya bulunursa) Stremio'da kabul edilen altyazı. **2026-10-30:** gerçek İngilizce tx3g MP4 bulunamadı; ffmpeg ile üretilen 4 dosyada (`moov` sonda / başta, öndeki boşlukla kaydırma, QuickTime `.mov`) cetvel ffmpeg'in geri çıkardığı zamanlarla birebir (120/120 satır, 0 ms), parçalı dosya "biçim dışı", altyazısız dosya "İng. iz yok" (`docs/spikes/k4-non-mkv.md`, `cargo run -- mp4-check`). Stremio'da uçtan uca deneme kalıyor
- [ ] **SS-034** Gömülü İngilizcesi olmayan MKV dışı dosyalar (çoğu MP4/AVI) için yol kararı: referans İngilizce altyazı (hash / birebir sürüm adı) mı, desteklenmez mi — SS-032 ölçümünden sonra; kanıt: `docs/decisions.md` satırı

## Çeviri yedeği (M6'dan öne çekildi)

> M5 bir istek için kabul edilen aday sunamıyorsa, aynı kaynaklardan bir
> İngilizce aday bulunur, cetvele (metin ya da PGS) hizalanır ve hedef dile
> çevrilir. 2026-10-30'da metin kaynağı gömülü izden dış İngilizce adaya
> çevrildi: blok okuma (SS-028) yavaş ve kayıplı, PGS'li dosyaları da
> kapsamıyordu. Karar: `docs/decisions.md` (2026-10-30, çeviri yedeğinin
> metin kaynağı). Sıra: SS-036 → SS-037 → SS-038 → SS-026.

- [x] **SS-028** Spike: İngilizce metin izinin blok metnini Cues konumlarından (`CueClusterPosition` + `CueRelativePosition`) hedefli oku; satır başına Range ile çoklu aralık (`bytes=a-b,c-d,…`) Real-Debrid'de karşılaştırılır. Metin loglanmaz, yalnız sayılar — kanıt: `docs/spikes/m6-block-text.md` (`with_relative`, istek sayısı, bayt, süre, okunan/beklenen satır; 5+ gerçek dosya). 2026-10-29: 5 bölümde konumlar doğru (`missing` 0, bulunan yükün tamamı UTF-8), RD çoklu aralık vermiyor; ama okumaların %39'u başarısız, 13–56 sn. İki filmde başarısızlıkların %97'si `connect` (bağlantı reddi). Düzeltme (4 eşzamanlı istek, 0.5/1.5/4 sn bekleyip tekrar) sonrası bir bölümde 4000 bloğun %96'sı okundu, saniyede ~5.4 blok (bölüm ~1.5–2 dk, film ~6–7 dk). **Sonuç: metin Cues konumlarından okunabiliyor; SS-026'ya geçilebilir** (diyalog izi seçimi ve kalan ~%4 blok orada ele alınır)
- [~] **SS-036** İngilizce yedek aday: hedef dilde aday yoksa ("aday yok") ya da hiçbiri kabul almazsa ("uyan yok"), aynı kaynaklarda `eng` aranır ve aynı sıralama, hizalama ve kabul testinden geçirilir. Kabul edilen en iyi aday, fix uygulanmış olarak menüde `eng` girdisi olur; hedef dilin nedeni yine gösterilir, cevap 1 gün geçerli (hedef dilde aday sonra çıkabilir). Etiketleme modunda ve `SUBSYNC_LANG=eng` iken kapalı. Çeviri (SS-037/SS-038) bu girdinin metnini kullanır — kanıt: testler (sahte kaynaklarla: yedek yalnız gerektiğinde aranır, kabul edilen İngilizce sunulur, reddedilen sunulmaz) + Stremio'da Türkçesi olmayan bir dosyada senkron `eng` girdisi
- [x] **SS-042** Spike: Stremio altyazı dosyasını (`/sub/<girdi>/<n>.srt`) her seçimde yeniden çekiyor mu? Liste oynatma başında bir kez isteniyor; dosya her seçimde çekiliyorsa sabit bir "çeviri" girdisi o ana kadar çevrilen kısmı döndürebilir ve SS-026 ikinci açılışı beklemez. Sunucu her dosya isteğini panelde `fetch` olayı olarak gösterir (girdi, sıra, bayt; metin yok) ve yanıta `Cache-Control: no-store` ekler. Deneme: bir oynatmada aynı girdiyi seç, kapat, yeniden seç; başka girdiye geç, geri dön; videoyu ileri sar — kanıt: `docs/spikes/` altında ölçüm notu (istemci başına kaç seçimde kaç çekme) → **2026-10-30 Apple TV:** liste bir kez istendi, dosya ise 4 adımda 4 kez çekildi (kapat → seç ve başka girdi → geri dön yeni istek üretti); yani seçimde o ana kadar çevrilen kısım döndürülebilir. Uzayan içeriğin oynatıcıda görünmesi, masaüstü ve Android TV SS-026'nın gerçek denemesinde (`docs/spikes/k5-subtitle-refetch.md`)
- [x] **SS-037** Çeviri çekirdeği: Nen'in `nen-translate` parçalarını taşı (bağlam pencereli bloklar, iki aşamalı belge analizi, yerel doğrulama, sınırlı onarım); `checkpoint`, `identity` ve WebVTT `artifact` taşınmaz, yerlerini M5 cache'i ve SRT yazıcısı alır. Girdi SS-036'nın kabul edilen İngilizce satırları; modele yalnız `cue_id + metin` gider, zamanlar yerelde birleştirilir. Çevirici bir trait — kanıt: sahte çeviriciyle testler (bozuk/eksik yanıt onarılır ya da reddedilir, zamanlar değişmez, sağlayıcı ağı yok) → **2026-10-30:** `src/translate.rs`, 13 test (142 toplam). Taşınanlar: 40 satırlık bloklar (6 satır yalnız bağlam, her satır tek bloğun çıktısı), tekrar eden özel adlar (en çok 32), belgenin tamamıyla bir kez analiz (özet, karakterler, sözlük; boş alan → ret), yanıt doğrulama (eksik, fazla, tekrar, boş) ve onarım bütçesi (2 hedefli + 1 tam deneme, yarım blok dönmez). Eklenen: satır içi boş satırlar atılır, `-->` içeren metin onarıma gider (SRT'yi bozmasın). `Translator` trait'i async (`Pin<Box<dyn Future>>`, kaynak trait'leri gibi); iptal future'ı düşürmekle olur, Nen'in iptal kapısı taşınmadı. Biten her blok çevrilmiş satırların ön ekini `on_block`'a verir (SS-042: seçimde o ana kadarki kısım sunulabilir). Prompt metni SS-038'de
- [x] **SS-038** OpenRouter adapter'ı (`OPENROUTER_API_KEY`, model ortam değişkeninden; anahtar ve metin loglanmaz; Nen'in istek/yanıt biçimi ve yeniden deneme davranışı) — kanıt: sahte HTTP ile testler + gerçek bir bölümün çevirisi (yalnız satır sayısı, token ve süre not edilir). **2026-10-30:** `src/openrouter.rs`, 7 test (149 toplam; sahte sunucu yalnız 127.0.0.1'de). Nen'den aynen: sistem talimatları, JSON istem girdisi (`promptVersion`/`schemaVersion` 2), katı JSON şeması (çıktı id'leri `enum`, tam sayı kadar öğe), ret ve fazla alan → kalıcı hata, 2 yeniden deneme (408/429/5xx ve bağlantı hatası; `Retry-After` en çok 10 sn, yoksa 0.5 / 2 sn), gövde sınırı 1 MiB. Farklı: model ön kontrolü yok (`require_parameters` zaten yapılandırılmış çıktısız modeli reddediyor), yalnız-OpenAI yönlendirmesi yerine `data_collection: deny`, çağrı süresi 120 sn. Varsayılan model `openai/gpt-6-luna` (kullanıcı seçimi; milyon token 0,10 $ giriş / 0,50 $ çıkış), `SUBSYNC_TRANSLATE_MODEL` ile değişir. Her çağrı sayılarla loglanır (`translate` alanı: süre, token, tekrar; anahtar ve metin yok). Anahtar `scripts/dev.sh openrouter` ile Anahtar Zinciri'nde; gerçek deneme `scripts/dev.sh translate-check <ingilizce.srt> [cikti.srt]` (ekrana yalnız sayılar). **2026-10-30 gerçek deneme:** 1 sa 50 dk'lık bir filmin 2 034 satırı 60 blokta, 802 sn'de, 0,0606 $'a çevrildi; ilk 37 satır 36 sn'de hazır, blok başına ortanca 11 sn, onarım ve tekrar 0, zamanlar kaynakla birebir. İlk denemede 25. blokta 200'den sonra gövde 120 sn'de gelmedi ve iş tekrar denenmeden düştü; kesilen gövde artık yeniden deneniyor (test eklendi). Çeviri oynatmanın ~8 katı hızlı (`docs/spikes/m6-openrouter.md`)
- [ ] **SS-026** Çeviri yedeğini teslime bağla: SS-036 İngilizce kabul edince çeviri işi arka planda başlar (dosya başına tek iş), bitince "otomatik çeviri" etiketli hedef dil girdisi M5 cache'ine yazılır (30 gün); iş sürerken menüde "çeviriliyor" nedeni — kanıt: sahte çeviriciyle testler + gerçek bir bölümde Stremio'da görünen çeviri. Tasarım notları: Stremio listeyi oynatma başında bir kez ister, bu yüzden hazır çeviri ikinci açılışta cache'ten gelir. Apple TV dosyayı her seçimde yeniden çekiyor (SS-042): menüde sabit bir girdi, seçildiğinde o ana kadar çevrilen kısmı SRT olarak döndürür; ikinci açılış beklenmez. Sıradaki bölümü önceden çevirmek ayrı iş

## Bakım

- [ ] **SS-041** Aynı oynatma için eşzamanlı istekleri tek probe'da birleştir: Stremio bir oynatmada iki altyazı isteği gönderiyor (önce `videoHash` yok, sonra var); ikisi de K2/M3/hizalamayı baştan yapıyor ve dump'ı iki kez yazıyor (2026-10-30, iki ayrı oynatmada görüldü). İstek anahtarı aynıysa (`kind/id/filename/size`; hash anahtarda yok) ikinci istek süren işi bekler — kanıt: test (aynı anda iki istek → tek arama, tek dump, iki yanıt da aynı menü) + gerçek oynatmada logda tek `m3 probe`

- [x] **SS-035** Varsayılan port 7000 macOS AirPlay Alıcısı (ControlCenter, `*:7000`) ile çakışıyor: 2026-10-30'da `scripts/dev.sh` "Address already in use" ile kapandı; aynı gün sonraki denemede 7000'e bağlanabildi, yani çakışma aralıklı. Varsayılan `127.0.0.1:7001`'e taşındı (`main.rs`, `dev.sh`, README, Caddyfile, deploy notu); port doluysa sunucu `SUBSYNC_BIND` ile başka port seçmeyi söyler. Yerel `127.0.0.1:7000` kurulum linki Stremio'da yeniden kurulmalı (`scripts/dev.sh link`); Funnel linki etkilenmez — kanıt: 126 test + AirPlay Alıcısı açıkken `scripts/dev.sh` ilk denemede 7001'de açıldı; 7001 doluyken ikinci kopya yeni mesajla `exit=1` verdi

- [x] **SS-029** Arka planda dönenleri görmek için canlı olay paneli: sunucunun log satırları (yalnız sayı ve etiket; token, link, kimlik, dosya adı, metin yok) son 1000 olay olarak bellekte de tutulur; `/<token>/dashboard` sayfası `events.json`'u 2 sn'de bir okuyup sayaçları, son istekleri adım adım (dosya eşleşmesi, metin cetveli, PGS, aday, hizalama, menü) ve süzülebilir ham olay akışını gösterir. Panel yoklamaları erişim loguna yazılmaz — kanıt: 108 geçen test (yanlış token 404, sayfada token yok, `after` ile yalnız yeni olaylar, tampon taşması) + yerel test sunucusunda panelin açılıp canlı güncellendiği, telefon genişliğinde taşmadığı görüldü. Her istekte dosyanın MKV olup olmadığı ve gömülü altyazılar okunur biçimde görünür (dil + biçim; "cetvel ✓", "PGS ölçülür", "dizinsiz", zorunlu); cache'ten verilen cevapta dosyanın yeniden okunmadığı yazar. Örnek satırlarla tarayıcıda denendi; satır biçimi `mkv` testinde sabitlendi. Gerçek bir oynatmada adımların doğru ayrıştırıldığı henüz görülmedi
- [x] **SS-030** Yerel sunucuyu tek komutla aç: token ve AIOStreams linki ilk çalıştırmada macOS Anahtar Zinciri'ne bir kez kaydedilir, kurulum linki her açılışta aynı kalır (her seferinde yeni token + yeniden kurulum yok). `scripts/dev.sh` (çift tıklanabilir `.command` ile) — kanıt: deneme Anahtar Zinciri girdisiyle betik denemesi + Stremio'da aynı linkle iki ayrı açılış → **2026-10-29 doğrulandı:** betik deneme Anahtar Zinciri girdisiyle denendi (ikinci açılış soru sormadı, token aynı kaldı, token log'a düşmedi); gerçek oturumda sunucu birçok kez yeniden açıldı, Stremio'daki kurulum aynı linkle çalışmaya devam etti
- [x] **SS-031** Olay panelinde her isteğin dosya adını göster; terminal log'u, dump ve cache adsız kalır. Senkron geri bildirimini isteğe bağlamak için (karar: `docs/decisions.md`, 2026-10-29) — kanıt: test (dosya adı `events.json`'da var, log satırında yok) + gerçek oynatmada panelde adın görünmesi → **2026-10-29 doğrulandı:** test geçiyor; kullanıcı gerçek oynatmalarda panelde dosya adlarını gördü
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
- **Öncelikler (2026-10-30):** çeviriden önce senkron ve aday seçimi
  güçlendirildi: SS-039 (basamaklı kaymada yerel ikinci şans) ve SS-017
  (SubDL ikinci kaynak) bitti. SS-040 (fazladan aday indirme) ölçüldü,
  kazancı küçük. SS-006 VPS/HTTPS bitti (2026-10-30). SS-042: Apple TV altyazı
  dosyasını her seçimde yeniden çekiyor. Sırada: SS-036'nın Stremio kanıtı,
  sonra çeviri yedeği (SS-037 → SS-038 → SS-026). SS-014 gerçek
  etiketleri artırma, SS-019 CD1/CD2 birleştirme de açık.
- **MKV dışı (2026-10-30):** SS-032/SS-033 kodu ve testleri hazır (126 test):
  kapsayıcı panelde görünür, MP4'te İngilizce `tx3g` izi varsa cetvel olur.
  Sırada kullanıcının MKV olmayan dosyasını açıp `docs/spikes/k4-non-mkv.md`
  tablosunu doldurmak; SS-034 kararı bu tabloya göre.
- **Çeviri yolu (2026-10-30):** metin kaynağı dış İngilizce aday (karar
  kaydı). Sıra SS-036 İngilizce yedek aday → SS-037 çekirdek → SS-038
  OpenRouter → SS-026 teslim. Gömülü blok okuma (SS-028) planda yok.
  PGS cetveli zamanı verdiği için yalnız PGS içeren dosyalar da kapsamda.
- **M2 konum ölçümü:** ana ölçüm tablosu `with_relative` sayısını kaydetmedi;
  paralel M2 kaydı 5/5 dosyada bulunduğunu bildiriyor. İki kayıt da
  `docs/spikes/m2-ruler.md` içinde korunur; aynı dosyalar olabilir. Bu kayıt,
  gerçek blok okuma başarısı ve maliyeti için SS-028'in yerini tutmaz.
- **SS-014:** 4 kullanıcı etiketi: 3 doğru kabul, 1 yanlış ret, 0 yanlış
  kabul. 2026-10-29 oturumunda (5 bölüm + 3 film) kullanıcı SubSync
  altyazılarında kayma görmedi; %50–60 isabetli kabuller dahil (zayıf etiket,
  `docs/spikes/m4-align.md`). Yerel senkron (SS-021) fayda göstermedi; yalnız etiketleme modunda.
  `cargo run -- calibrate dumps` yerel zaman kayıtlarını yeniden değerlendirir.
- **Sunucu:** proje kökünde, ortam değişkenlerinin hazır olduğu terminalde
  `SUBSYNC_DUMP_DIR=dumps cargo run`. `SUBSYNC_TOKEN` (en az 32 karakter) ve
  `SUBSYNC_AIOSTREAMS_URL` gerekir. Range ve kaynaklar varsayılan açık,
  `SUBSYNC_LANG=tur`; SubDL yalnız `SUBDL_API_KEY` varsa açılır.
- **Olay paneli:** sunucu açıkken `http://127.0.0.1:<port>/<token>/dashboard`
  (Funnel üzerinden de aynı token'la açılır). Yalnız bellekte; yeniden
  başlatınca boşalır. Satırlar loglanan metnin aynısıdır.
- **Blok ölçümü:** aynı komuta `SUBSYNC_BLOCK_PROBE=1` ekle. Probe arka planda
  çalışır. Cache'ten yanıtlanan dosyada probe çalışmaz; ölçüm için daha önce
  açılmamış dosyalar seç. Çıktı `m6 blocks`, yalnız sayı ve tür taşır.
- **Yerel veriler:** 7 cache ve 14 dump dosyası hem köke hem
  `.local/branch-backups/` altına kopyalanıp doğrulandı. Eski dallar
  `codex/archive/` altında; worktree'ler açık Claude süreçleri kullandığı
  için silinmedi. Güncel geliştirme kökteki `main` üzerinde yapılır.
  Ayrıntı birleştirme kaydında.
- **Yayın:** kalıcı kurulum Oracle ARM sunucusunda (SS-006, `deploy/deploy.sh`,
  `docs/deploy.md`). Yerel deneme için Tailscale Funnel (443 → `127.0.0.1:7001`,
  `tailscale funnel 7001`); kapatmak: `tailscale funnel --https=443 off`.
- **Açık keşif:** SS-007 TorBox (abonelik bekliyor; URL
  çağrısı indirme başlatabilir, ilk gerçek deneme kullanıcı onayıyla).
- **Güvenlik notu:** SubSync token'ı sunucuda yeniden üretildi (SS-006);
  AIOStreams şifresi keşif ölçümleri bitince değiştirilmeli.
  Linkler sohbete veya loga yazılmaz.
