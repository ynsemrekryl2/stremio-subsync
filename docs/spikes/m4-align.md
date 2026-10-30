# M4 — Hizalama çekirdeği: kendimiz mi, `alass-core` mu?

Soru (SS-011): Aday altyazıyı cetvele oturtan çekirdeği kendimiz mi yazalım,
yoksa `alass-core` 2.0.0'ı (GPL-3.0) mı kullanalım?

## Bizim çekirdek (`src/align.rs`)

1. **Arama:** Her fps adayı için (1, 25↔23.976, 24↔23.976, 25↔24) aday
   satırlar ölçeklenir. 10 ms ızgarada ±10 dk kaydırma boyunca toplam örtüşme
   tam olarak hesaplanır: bir satır çiftinin örtüşmesi kaydırmanın yamuk
   biçimli bir fonksiyonu olduğu için çift başına dört eğim değişimi ve iki
   kümülatif toplam yeterli. FFT yok, bağımlılık yok.
2. **İnce ayar:** En iyi (ölçek, kaydırma) bulunduktan sonra, cetvel satırına
   300 ms içinde başlayan aday satırların başlangıç farklarının ortancası
   kaydırmaya eklenir (1 ms çözünürlük).
3. **Kabul testi:** Cetvel süresi baş / orta / son diye üçe bölünür. Her
   pencere için:
   - en az 20 cetvel satırı (`ThinWindow`),
   - pencerenin kendi en iyi kaydırması (global ±60 sn içinde aranır)
     globalden en fazla 300 ms uzakta (`Disagree`),
   - aday satırı sayısı cetvel satırlarının en az %40'ı (`Sparse`; CD1 gibi
     yarım altyazılar),
   - satırların en az %50'si 300 ms içinde karşılanıyor; oran **satırı az
     olan tarafa** göre (`LowHits`). İlk sürüm cetvele bölüyordu; satırları
     birleştiren çeviriler haksız yere düşüyordu (aşağıda Bölüm 1).
   Biri tutmazsa altyazı sunulmaz.

Eşikler sentetik veriyle seçildi; gerçek dosyalarla SS-014'te ayarlanacak.

## Sentetik etiketli set (`src/synth.rs`)

Tohum başına bir film (85–130 dk konuşma izi, ara sıra 30–150 sn sessiz
sahne). Cetvel: ±40 ms oynama. Aday: ±150 ms oynama, %8 eksik satır, %5 ses
efekti satırı, %12 birleştirme.

| Durum | Etiket | Nasıl üretildi |
|---|---|---|
| `same`, `shifted` | kabul | aynı zamanlama / ±90 sn kaydırma |
| `pal-fast`, `pal-slow`, `ntsc24` | kabul | fps farkı + ±30 sn kaydırma |
| `other-film` | ret | başka bir filmin konuşma izi |
| `other-cut` | ret | ortadaki %25–75'lik bölümde yalnız adayda olan 20–90 sn'lik sahne |
| `partial` | ret | aday filmin yalnız ilk %45'ini kapsıyor |
| `drift` | ret | listede olmayan 1.003 ölçek (2 saatte ~20 sn kayma) |
| `scrambled` | ret | her satır ±2.5 sn rastgele kaymış |
| `forced-ruler` | ret | cetvel yalnız ~30 satırlık zorunlu iz |
| `translated`, `translated-pal` | kabul | 4 sn'ye kadar aralıklı satırların %60'ı birleşik; satır sayısı cetvelin %60–73'ü (gerçek bölümde %62) |
| `cd1` | ret | çevirinin yalnız ilk %55'i (iki CD'lik altyazının ilk parçası) |

## Karşılaştırma (2026-10-27)

20 tohum × 11 durum = 220 hizalama, `--release`, Apple Silicon. `alass-core`
tarafı: `align_nosplit` + `overlap_scoring`, aynı 7 fps adayıyla (alass-cli'nin
fps yöntemi); bölmeli mod ayrıca `align(…, 7.0, Some(3.0), standard_scoring)`.
Betik repoda değil (geçici bir crate, `#[path]` ile aynı modülleri kullandı).

| Durum | Bizim kabul | Bizim fps doğru | Bizim hata (ortanca) | alass fps doğru | alass hata (ortanca) | alass bölmeli: farklı kaydırma sayısı (ortanca) |
|---|---|---|---|---|---|---|
| same | 20/20 | 20/20 | 4 ms | 20/20 | 3 ms | 1 |
| shifted | 20/20 | 20/20 | 3 ms | 20/20 | 3 ms | 1 |
| pal-fast | 20/20 | 20/20 | 3 ms | 20/20 | 3 ms | 11 |
| pal-slow | 20/20 | 20/20 | 3 ms | 20/20 | 3 ms | 9 |
| ntsc24 | 20/20 | 20/20 | 3 ms | 20/20 | 4 ms | 7 |
| other-film | 0/20 | — | — | — | — | 6 |
| other-cut | 0/20 | — | — | — | — | 2 |
| partial | 0/20 | — | — | — | — | 1 |
| drift | 0/20 | — | — | — | — | 16 |
| scrambled | 0/20 | — | — | — | — | 1 |
| forced-ruler | 0/20 | — | — | — | — | 31 |

| | ms / hizalama |
|---|---|
| Bizim (7 fps adayı + kabul testi) | 3.4 |
| alass `align_nosplit` (7 fps adayı, kabul testi yok) | 90.9 |
| alass bölmeli (tek ölçek) | 107.1 |

## Sonuç

- Doğruluk aynı: iki taraf da her pozitif durumda fps'i doğru buldu, kaydırma
  hatası ortanca 3–4 ms.
- Bizimki ~27 kat hızlı ve kabul testini de içeriyor. `alass-core` ret kararı
  vermiyor; bölmeli modu başka bir filme ya da kaymaya bile birkaç parçalı bir
  "çözüm" üretiyor. Kabul testini yine kendimiz yazmamız gerekirdi.
- `alass-core` GPL-3.0; bağımlılık olarak eklemek binary'yi GPL yapar.
- `alass`'ın tek artısı: farklı kurgu (`other-cut`) dosyalarını bölmeli modla
  düzeltebilmesi (ortanca 2 kaydırma). Bunu şimdilik reddediyoruz; ileride
  istenirse M6 adayı.

**Karar:** Çekirdek bizim. `docs/decisions.md`'ye yazıldı.

## Gerçek dosyalar (2026-10-27, `m4 probe`)

Cetvel M2 ölçümündeki dosyalar; aday OpenSubtitles `tur`. "Etiket" kullanıcının
izleyerek verdiği karardır; boşsa henüz etiketlenmedi.

| Dosya | Aday | Fix | Pencere kaydırmaları (global'e göre) | İsabet (baş / orta / son) | Karar | Etiket |
|---|---|---|---|---|---|---|
| Film 2 (~160 dk) | 668 satır, ~89 dk'da bitiyor | 1000/1001, −43.5 sn | −24, −94 ms, yok | %16 / %30 / %0 | ret (`LowHits(0)`) | — |
| Bölüm 1 (~44 dk) | 619 satır | 1/1, 0 ms | 0, 0, +40 ms | %32 / %40 / %35 | ret (`LowHits(0)`) | — |

- **Film 2:** CD1 parçası; son pencerede aday yok. Ret doğru (bkz. SS-019).
- **Bölüm 1:** Üç pencere de aynı kaydırmada (0 ms, kayma yok) buluşuyor; rastgele
  bir aday için bu neredeyse imkânsız. Büyük olasılıkla doğru senkronlu ama
  isabet eşiği (%50) yüzünden reddedildi: **yanlış ret adayı.**
  İsabet cetvel satırına bölünüyor; aday 619, cetvel 1006 satır. Türkçe
  çeviri satırları birleştirdiği için tam uyumda bile en fazla ~%61 olabilir.
  Adayın satırlarına göre oran ~%58.
- Sentetik sette aday ve cetvel satır sayıları yakındı; bu yanlılık orada
  görünmedi.

### İkinci tur (2026-10-27 gece, yeni ölçütle, `calibrate dumps`)

7 istek → 4 dump; biri aynı dosyanın tekrarı, yani **3 farklı dosya**. Diğer
3 istekte cetvel yoktu (2'sinde K2 eşleşmedi, 1'inde İngilizce metin izi yok).
Satırlar: cetvel/aday satır sayısı; isabet ve kapsama pencere başına.

| Dump | İz | Aday (puan) | Satır | Karar | Fix | İsabet | Pencere kaydırmaları | Etiket |
|---|---|---|---|---|---|---|---|---|
| …938389 (bölüm) | 5 | 0 (10) | 1006/619 | kabul | 1/1, 0 ms | %51 / %65 / %60 | 0, 0, +40 | — |
| …057530 (film A) | 4 (1835 satır) | 0 (97) | 1835/1359 | ret `LowHits` | 1/1, +362 ms | %42 / %37 / %43 | −82, +278, +168 | — |
| | | 1 (81) | 1835/1498 | kabul | 1/1, −34 ms | %99 / %98 / %99 | +24, +24, +24 | — |
| | | 2 (22) | 1835/1359 | kabul | 1001/1000, +1.6 sn | %63 / %62 / %57 | +43, +93, +113 | — |
| | 3 (1498 satır) | 0, 1, 2 | | aynı kararlar | | | | |
| …242878 (film B) | 9 | 0 (64) | 2312/1652 | kabul | 1/1, −43 ms | %84 / %77 / %89 | −37, −67, −7 | — |
| | | 1 (33) | 2312/1881 | kabul | 1001/1000, +0.7 sn | %55 / %52 / %56 | +31, +61, +61 | — |
| | 3 (25 satır, forced) | 0, 1 | | ret `ThinWindow` | | | | |

- **Bölüm 1 artık kabul** (ölçüt düzeltmesi işe yaradı), ama baş penceresi
  %51 ile eşiğin hemen üstünde.
- Film A'da aday 1, cetvelin 3. iziyle **birebir aynı satır sayısında**
  (1498) ve %99 isabetli: büyük olasılıkla aynı İngilizce altyazıdan çevrilmiş.
- Film A'da **adı en çok benzeyen aday (puan 97) reddedildi**: kaydırmalar
  pencereden pencereye 360 ms oynuyor, isabet %40. Ya zamanlaması özensiz ya
  da başka bir kurgu; etiket gerekli. Sıralama puanı tek başına doğruluk
  göstergesi değil.
- İki İngilizce izi olan dosyada (normal + SDH) kararlar iz değişince
  değişmedi; SDH'de kapsama düşük, isabet benzer.
- Sınırdaki kabuller (%52–61 isabet) etiketlenmeden eşik değiştirilmemeli.

### Etiketler

| Dump | Aday | Karar | Fix | Pencere kaydırmaları (global'e göre) | İsabet | Kullanıcı etiketi |
|---|---|---|---|---|---|---|
| …265984 (bölüm, ~22 dk) | 1 (puan 75), 532 satır / cetvel 551 | ret `Disagree(0)` | 1/1, −500 ms | −450, +50, +60 | %36 / %46 / %28 | **kullanılabilir**: ortada çok az erken, başta biraz daha erken → **yanlış ret** (temkinli yönde) |
| …055228 (bölüm) | 1 (puan 75), 524 satır / cetvel 687 | kabul | 1/1, +233 ms | +47, −43, +47 | %96 / %96 / %98 | **senkron** (baş, orta, son) |
| | 2 (puan 75), 524 satır | kabul | 1/1, **−12.8 sn** | +47, −43, +47 | %96 / %96 / %98 | **senkron** |
| | 3 (puan 64), 524 satır | kabul | 1/1, **+4.1 sn** | +43, −47, +43 | %96 / %96 / %98 | **senkron** |

- **…055228:** Aynı çeviri üç farklı sürüm için yayınlanmış (hepsi 524
  satır); kaydırmalar −12.8 sn ile +4.1 sn arasında. Üçü de kabul edildi ve
  kullanıcı üçünü de baş, orta ve sonda senkron buldu. Kaydırma düzeltmesi
  büyük farklarda da doğru çalışıyor.
- **Şu ana kadar:** 4 etiketli aday; 3 doğru kabul, 1 yanlış ret (temkinli
  yönde), **0 yanlış kabul**.
- Orta ve son pencere −450 / −440 ms diyor; global −500 ms bunlardan ~60 ms
  erken. Kullanıcının "çok az erken" demesi bununla uyumlu.
- Baş penceresi −950 ms diyor: ilk ~7 dakikada altyazı −500 ile ~0.45 sn geç
  kalıyor olmalı. Baş gerçekten kaymışsa ret doğru; değilse baş penceresinin
  tahmini yanlış ve bu bir yanlış ret.
- İsabet düşük (%28–46) ama satır sayıları neredeyse eşit: bu çevirinin satır
  başlangıçları İngilizce izden belirgin farklı zamanlanmış.
- Kullanıcı başı **daha erken** buldu; baş penceresi tahmini ise altyazının
  başta **geç** kaldığını söylüyordu. Yani baş penceresinin −450 ms'lik
  sapması gerçek değil, tahmin gürültüsü.
- Kullanıcı notu: kayma baştan sona **azalıyordu**; altyazı içerik olarak
  doğru, birkaç noktada ayrı senkronlansa kullanılır. Altıya bölünce her
  parçada gereken ek kaydırma: −690, −348, −216, −95, −462, −599 ms (eşleşme
  her parçada ~%45). Ölçüm ne düzenli bir azalma ne de kullanıcının gördüğü
  yön (erken) ile uyuşuyor; kabul edilen …055228'de aynı ölçüm her parçada
  ±4 ms. Bu adayın satır zamanları İngilizce izle gevşek ilişkili; parçalı
  senkron burada gürültüyü kovalardı. Fikir SS-021'e yazıldı.
- Deneme: pencere kaydırmasını örtüşme tepesi yerine satır başlangıçlarının
  en yoğun ±300 ms kümesinden hesaplamak. Sentetik set geçti ama gerçek
  dosyalarda kararlar değişmedi; bu aday yine `Disagree` (−639, −95, −565 ms)
  ve iyi adaylarda tutarlılık belirgin artmadı. **Geri alındı.**
- Ders: isabet ~%45'in altındayken pencere kaydırma tahminleri ±0.5 sn
  oynuyor; bu bölgede ret kararı güvenilir ama "gerçekten kayıyor" anlamına
  gelmiyor. Eşikleri gevşetmek için daha çok etiket gerekli, özellikle
  isabeti %35–55 olan adaylar.

### Teslim sırasında bulunanlar (2026-10-28)

- **Yanlış ret, iki İngilizce izli bölüm (…441642):** Aday iz 3'e karşı %99
  isabetle kabul, iz 4'e (SDH, 569 satır) karşı reddedildi: baş penceresinin
  örtüşme tepesi −57.8 sn'ye kaçmış, oysa o pencerede isabet %98. Menü yalnız
  ilk ize bakıyordu, "uyan yok" yazdı. Düzeltmeler:
  - Menü, herhangi bir İngilizce ize karşı kabul edilen adayı sunar.
  - Pencere, global fix'te isabeti ≥ %80 ise kendi örtüşme tepesi nerede
    olursa olsun "uyuyor" sayılır (`STRONG_HIT_RATE`). Uzun SDH satırları
    tepeyi bir dakika kaydırabiliyor.
  - Etkisi: tüm dump'larda yalnız bu deneme ret → kabul oldu; sentetik
    negatiflerin hepsi hâlâ ret, etiketli ret (…5984) hâlâ ret.
- **İngilizce altyazısı PGS olan film:** "İngilizce iz yok" demek yanıltıcıydı
  (Stremio İngilizce altyazıyı gösteriyor). Ayrı neden: "⚠ İng. resim".
  PGS'ten cetvel çıkarma fikri SS-025.

### Kapsama (2026-10-28, tüm oturumların logu)

21 farklı dosya (log izi/Cues boyutuyla ayırt edildi; ad yok):

| Durum | Dosya | Pay |
|---|---|---|
| Cetvel var (İngilizce metin izi) | 10 | %48 |
| İngilizce yalnız resim (PGS) | 6 | %29 |
| K2: dosya listede eşleşmedi | 3 | %14 |
| İngilizce hiç yok | 1 | %5 |
| İngilizce metin var, dizinlenmemiş | 1 | %5 |

Türkçe aday bulunamayan arama: 38 aramanın 3'ü. Asıl kayıp cetvelde:
dosyaların yarısında cetvel yok. En büyük iki kazanç adayı: PGS'ten cetvel
(SS-025; SRT + PGS içeren iki dosyada PGS cue noktası SRT satırının ~2 katı:
2008 → 4020, 1061 → 2406, yani gösterme + temizleme çiftleri) ve K2'de
boyutla eşleşme (SS-020; eşleşmeyen bölümde boyut 2 stream'de tutuyordu).

### Kalibrasyon düzeni

`SUBSYNC_DUMP_DIR=dumps` ile her hizalanan istek `dumps/<unix ms>.json`
olarak saklanır: yalnız cetvel ve aday satır zamanları ile sıralama puanı
(kimlik, ad, metin yok; `dumps/` git'e girmez). `cargo run -- calibrate dumps`
hepsini güncel kodla yeniden hizalar ve deneme başına bir satır basar.

### Etiketleme modu

`SUBSYNC_LABEL_MODE=1` açıkken altyazı isteği probe'ları en çok 15 sn bekler
ve indirilen her adayı Stremio menüsünde listeler: `SubSync <sıra> ✓|✗
<kaydırma>[ fps] · <dump eki>`. Sıra `calibrate`'teki `cand` + 1; dump eki
dump dosya adının son 4 hanesi. Kabul edilenler de reddedilenler de kendi en
iyi fix'iyle kaydırılmış gelir: ✗ işaretli biri senkronlu görünüyorsa o bir
yanlış ret. Altyazılar yalnız bellekte (son 32 istek).

**Yerel senkron (SS-021):** Kaydırması olan her aday menüde bir de `yerel`
sürümüyle gelir. Global fix'ten sonra her satır, 700 ms içindeki en yakın
cetvel satırıyla eşlenir (bir cetvel satırı yalnız en yakın adayla); satırın
ek kaydırması, iki yanındaki 10'ar satırın eşleşme farklarının ortancasıdır.
Yakınında 5'ten az eşleşme olan satır yalnız global fix'le kalır. Tek bir
yanlış eşleşme bir satırı fırlatamaz; 0.7 sn'den büyük yerel kayma
izlenemez. Sentetik testte 0.02%'lik kayma (100 dk'da 1.2 sn) ortanca
120 ms'nin altına iniyor.

**Yerel senkron sonucu (2026-10-28):** Kullanıcı ne reddedilen ne kabul
edilen adaylarda yerel sürümle fark gördü. `calibrate` artık yerel
senkronun satırları ne kadar kaydırdığını da basıyor:

| Dump / aday | Satırların kayan payı | Ortanca kayma | En büyük |
|---|---|---|---|
| …5228 / 0–2 (senkron, kabul) | %99 | 33 ms | 103 ms |
| …5984 / 0 (kayan, ret) | %99 | 165 ms | 517 ms |

- İyi altyazıyı bozmuyor (≤0.1 sn), ama kayan altyazıda da değişiklik
  0.05–0.28 sn; algı sınırında.
- Yön ters: …5984'te yerel senkron satırları **daha erkene** (−50…−280 ms)
  çekiyor; kullanıcı ise altyazıyı zaten erken görüyor. Türkçe satırlar
  İngilizce izden ~0.1–0.3 sn sonra başlıyor, demek ki bu dosyada **İngilizce
  iz sesten erken**. Cetvelin kendi hatası, ulaşılabilecek kesinliğin
  sınırı; ses kullanmadan bunu düzeltemeyiz.
- Karar: yerel senkron teslimde kullanılmayacak; yalnız etiketleme modunda
  kalıyor (SS-021).

Etiket sözlüğü: **senkron** (baştan sona tutuyor) · **kaymış** (sabit bir
gecikme var) · **kayıyor** (zamanla açılıyor) · **yanlış** (başka sürüm/kurgu,
yer yer tutmuyor).

## Gerçek oturum geri bildirimi (2026-10-29)

Masaüstü Stremio, 5 bölüm + 3 film. Kullanıcı: "açtığım SubSync
altyazılarından hiçbiri kayık değildi". Bu **zayıf etiket**: birden çok girdi
sunulduğunda hangisinin seçildiği bilinmiyor, bu yüzden yukarıdaki 4'lü sete
eklenmedi. Yine de sınırdaki kabuller için ilk gerçek işaret:

| İçerik | Kabul edilen aday | Fix | İsabet (baş / orta / son) |
|---|---|---|---|
| bölüm | 0 | 1/1, −699 ms | %52 / %52 / %53 (iki metin izinden biri ret `LowHits`) |
| bölüm | 0 | 1/1, −2345 ms | %61 / %61 / %58 |
| film | 1 · 2 | 1/1, −34 ms · 1001/1000, +1.6 sn | %99 · %63 / %62 / %57 |
| film | 0 · 1 | 1/1, +15 ms · 1001/1000, +0.75 sn | %83 / %79 / %88 · %53 / %50 / %55 |
| film (boyutla eşleşti) | 1 · 2 · (iz 4'te 0) | 1/1, +981 · +1082 · +384 ms | %96 · %100 · %84 |

Bir bölümde 3 aday da ret aldı (`Disagree` / `LowHits`, en iyi %36); menüde
"uyan altyazı yok" sunuldu. %50–60 isabetli kabullerde kayma bildirilmedi.
Eşik değişikliği için hâlâ seçilen girdinin bilindiği etiket gerekir; dosya
adı artık olay panelinde göründüğü için (SS-031) bu mümkün.

## Basamaklı kayma (2026-10-30, SS-036 denemesi)

Bir dizi bölümünde (gömülü İngilizce SRT, 506 satır) 3 Türkçe ve 3 İngilizce
adayın altısı da ret aldı; hepsinde orta pencere düştü (isabet ~%12,
`Disagree` / `LowHits`). Dump'ta cetvel ile Türkçe adaylar 2 dakikalık
dilimlerle karşılaştırıldı (global fix 25025/24000, +2.0–2.3 sn):

- Kesik yok; kayma **basamak basamak** değişiyor. En iyi adayda 4–16. dk
  ~+80 ms, 16–26. dk ~+350 ms, 26–30. dk ~+170 ms, 32–38. dk ~−125 ms,
  38–46. dk ~−270 ms. Üç adayda da aynı biçim, yani fark dosyanın kendisinde
  (yayın ile web sürümü arasında perde aralarının birkaç yüz ms farklı
  olması gibi). Toplam açıklık ~0.6–0.9 sn.
- SS-021'in yerel kaydırması (her satıra ±20 komşusunun 1 sn içindeki
  eşleşmelerinin ortancası) üç adayda da isabeti eşitliyor:

| aday | global ±400 ms | yerel ±400 ms | global ±200 ms | yerel ±200 ms | üçte birler, global → yerel (±400 ms) | yerel kaydırma aralığı |
|---|---|---|---|---|---|---|
| 0 | %59 | %84 | %32 | %72 | 78/25/78 → 83/84/85 | −587…+140 ms |
| 1 | %61 | %87 | %33 | %71 | 86/23/77 → 86/86/88 | −584…+302 ms |
| 2 | %74 | %84 | %45 | %72 | 82/62/77 → 82/83/86 | −416…+272 ms |

Sonuç: SS-021 kabul edilmiş dosyalarda fayda göstermemişti; burada tam
kullanım yerini buldu. Global olarak tutup yerel olarak tutmayan dosyalar
için ikinci şans: SS-039.

### İkinci şans ölçümü (2026-10-30, `align::local_check`)

Üç pencere, her aday satırı kendi yerel kaydırmasıyla taşındıktan sonra
yeniden sayıldı (isabet ±300 ms, pencere başına, global testle aynı sayım).
`calibrate` satırındaki `local=[… span … h…% h…% h…%]` bu değerleri basar.

**Gerçek dump'lar** (27 dosya, 102 deneme; 40 kabul, 62 ret). Yalnız
`Disagree` / `LowHits` retleri ikinci şansın konusu; `Sparse` (eksik aday,
CD1) ve `ThinWindow` (forced iz) ayrı nedenler, dokunulmaz.

| Dump | Aday | Global ret | Global isabet | Yerel isabet | Yerel aralık |
|---|---|---|---|---|---|
| …289899 (bölüm, bu not) | 0 · 1 · 2 | `Disagree` / `LowHits` | %12–43 orta | **%81–85** her pencerede | 751–970 ms |
| …662918 (bölüm, 3 iz) | 1 (1001/1000) | `LowHits(1)` | %48 orta | %73–74 en düşük | 811 ms |
| …057530 (film A) | 0 (puan 97) | `LowHits(0)` | %37–43 | %58–63 | 1130–1208 ms |
| …538153 (bölüm) | 1 · 2 | `LowHits(0)` | %42–51 | %50–55 | 640 ms |
| …265984 (etiketli: "kullanılabilir, gevşek") | 0 | `Disagree(0)` | %28–46 | %47–51 | 921 ms |
| diğer 30+ ret | | | ≤%40 | ≤%40 | |

Kabul edilenlerde yerel isabet %47–100, aralık 30–822 ms; yerel geçiş
kabulleri bozmuyor.

**Sentetik etiketli set** (32 tohum × 14 durum): `Disagree`/`LowHits` ile
reddedilen negatiflerde yerel en düşük pencere isabeti en çok **%36**
(other-cut), diğerlerinde ≤%17. Pozitiflerde en az **%91**. `forced-ruler`
negatifi yerelde %67–100'e çıkıyor ama globalde `ThinWindow` alıyor, yani
ikinci şansa girmiyor. Sette basamaklı kayma pozitif durumu yok; SS-039'da
eklenecek.

**Önerilen kural (SS-039):** global ret `Disagree` ya da `LowHits` ise ve
yerel geçişte üç pencerenin hepsi ≥ **%75** isabetliyse ve yerel aralık
≤ **1200 ms** ise kabul; altyazı `srt::render_local` ile sunulur. Bu kural
gerçek dump'larda yalnız …289899'un üç adayını kabule çevirir (…662918'in
1001/1000 adayı %73–74 ile dışarıda kalır; etiketsiz ve kardeş adaylar 1/1
fix aldığı için temkinli). Sentetik negatiflerle arada ~40 puan pay var.
Kurtarılan pay küçük (27 dosyanın 1'i), ama o dosyada hiçbir adayı
sunamıyorduk.

**Uygulandı (SS-039).** `calibrate` artık yerel kabulü `ACCEPT-local` diye
yazar; 27 dump'ta sonuç ölçümle aynı: 40 kabul değişmedi, …289899'un 3 adayı
`ACCEPT-local`, 59 ret. Sentetik `synth::stepped` (gerçek bölümdeki basamak
biçimi, %80–130 gerilmiş, yarısında ters işaretli, 16 tohum): 6'sı yerel
geçişle sunuluyor ve satırlar doğru yerine ortanca ~25 ms, p90 ≤ 85 ms
yakın düşüyor. Diğer 10'u **global testten zaten geçiyor** ve tek kaydırmayla
sunulunca ortanca 130–320 ms, p90 240–470 ms hatalı kalıyor. Yani hafif
basamaklı kaymada global kabul de kusursuz değil; kabul edilmiş adaylara da
yerel kaydırma uygulamak ayrı bir karar (SS-021'de kullanıcı fark görmemişti),
etiket olmadan açılmadı.

**Stremio'da (2026-10-30):** …289899'un bölümü yeniden açıldı; 3 aday
`accepted: true` (yerel isabet %81–85), menüde 3 `tur` girdisi. Kullanıcı
seçtiği girdiyi baş, orta (16–30. dk) ve sonda **senkron** buldu. Etiket:
yerel kabul doğru (1 bölüm, 3 aday).

## Açık

- Sentetik veri gerçekten kolay: pozitiflerde satırların %84–94'ü karşılanıyor,
  negatiflerde en çok %16 (tek pencerede değil, genelde). Gerçek altyazılar
  arasındaki farklar (satır bölme, SDH, farklı zamanlama alışkanlıkları) bu
  oranları düşürür. Eşikler SS-014'te gerçek etiketli setle ayarlanacak.
