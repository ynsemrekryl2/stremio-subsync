# M3 — Paralel dalın tarihsel kaynak ölçümü

Kaynak: `9d284a7` (`claude/m3e-basha-a4d3a4`). SS numaraları güncel iş
listesine eşlendi. Aşağıdaki puanlar, CD elemesi, ortam değişkenleri ve probe
davranışı eski uygulamaya aittir; güncel kullanım için [README](../../README.md) ve
[güncel kaynak ölçümü](m3-sources.md) geçerlidir. Hash extra'sı ve hash öncelikli sıralama güncel
istemciye aktarılmadı; gerçek hash ölçümü bu dalda da yapılmamıştı.
Eski kod Git geçmişinde korunur.

Soru (SS-016): OpenSubtitles v3 (Stremio'nun anahtarsız add-on'u) bir IMDb id
için hangi alanları veriyor; indirilen dosya hangi kodlamada; sıralama için
sürüm adı var mı?

## OpenSubtitles v3 (2026-10-27, herkese açık film/dizi ile)

İstek: `GET https://opensubtitles-v3.strem.io/subtitles/{type}/{id}.json`
(`id` = `tt…` ya da dizi için `tt…:sezon:bölüm`). Deneme: tt0111161 (film) ve
tt0944947:1:2 (dizi bölümü).

| Ölçüm | Film | Dizi bölümü |
|---|---|---|
| Durum / boyut | 200, 11,5 KB | 200 |
| Aday (tüm diller) | 37 | 92 |
| Türkçe (`lang: "tur"`) | 3 | — |
| `m` (eşleşme türü) | hepsi `i` (IMDb) | hepsi `i` |
| `season`/`episode` | 0/0 | hepsi 1/2 |

- Alanlar: `id`, `url`, `lang` (ISO 639-2: `tur`, `eng`, `pob`…), `SubEncoding`,
  `m`, `g`, `subtitleFileName`, `movieReleaseName`, `fpsMilli` (bazen),
  `releaseGroup` / `releaseFormat` / `releaseOther` (bazen), `season`, `episode`.
- **Sürüm adı var:** `subtitleFileName` (ör. `…_DVDrip_DivX-=Grup=-(CD1).srt`)
  ve kimi zaman `releaseGroup`/`releaseFormat`. Sıralama bunlarla yapılabilir.
- **Çok parçalı adaylar var:** `(CD1)` gibi adlar filmin yalnız bir kısmını
  kapsar; hizalanamaz, elenmeli.
- `filename=…&videoSize=…` extra'sı eklemek listeyi değiştirmedi (37 → 37,
  hepsi `m: i`). `videoHash` ile `m: h` (hash eşleşmesi) dönebilir; ölçülmedi.
- İndirme: `url` `subs5.strem.io/…/subencoding-stremio-utf8/…` biçiminde; tek
  istek, yönlendirme yok, `Content-Type: application/x-subrip; charset=utf-8`.
  `SubEncoding: CP1254` olan Türkçe dosya UTF-8'e çevrilmiş geldi (130 KB,
  CRLF, `00:01:51,978 --> …`). Kodlama tahmini gerekmiyor.
- Uzantılar çoğunlukla `.srt`; `.txt` olanlar da var (biçimi ayrıca ölçülecek).

## SubDL

Ücretsiz anahtar gerekiyor; kullanıcı sonra alacak (SS-017). Biçim anahtarla
ölçülecek.

## Aday toplama probe'u (SS-016)

`SUBSYNC_SOURCES_PROBE=1` (dil: `SUBSYNC_LANGUAGES`, varsayılan `tur`). Her
altyazı isteğinde yanıtı bekletmeden: OpenSubtitles v3 listesi → dil süzgeci →
çok parçalılar elenir → sıralama (hash eşleşmesi önce, sonra sürüm adı puanı)
→ ilk 3 sırayla indirilir → SRT zamanları okunur. Log satırı `m3 sources …`
yalnız sayı taşır; id, dosya adı, URL, sürüm adı ve metin yok.

Deneme (2026-10-27, yerel sunucu, `curl` ile; herkese açık başlıklar, uydurma
ama gerçekçi dosya adları, `videoHash` yok):

| # | Tür | Liste | Türkçe | CD elenen | İndirilen | SRT olayları | Geçersiz satır | Puanlar | Liste / indirme ms |
|---|---|---|---|---|---|---|---|---|---|
| 1 | film | 37 | 3 | 1 | 2 | 1685, 1683 | 0 | 75, 50 | 399 / 534 |
| 2 | film | 65 | 2 | 1 | 1 | 1619 | 0 | 66 | 237 / 122 |
| 3 | film | 90 | 7 | 1 | 3 | 1459, 1353, 1456 | 0 | 44, 44, 40 | 329 / 521 |
| 4 | bölüm | 92 | 5 | 0 | 3 | 578, 573, 569 | 0 | 253, 100, 60 | 215 / 667 |
| 5 | bölüm | 88 | 4 | 0 | 3 | 584, 578, 584 | 0 | 50, 50, 36 | 157 / 593 |
| 6 | film | 96 | 7 | 0 | 3 | 2794, 2833, 2662 | 3 / 2797 | 237, 57, 46 | 165 / 767 |

- 6/6 başlıkta Türkçe aday var (2–7). İndirilen 15 dosyanın 15'i UTF-8 SRT
  olarak okundu; geçersiz zaman satırı toplamda 3.
- Bir başlık için toplam süre ≈ 0,3–1,2 sn (liste + 3 indirme, sırayla).
- Aynı başlığın adayları farklı fps'li (23976 / 25000 / 29970): M4'teki fps
  düzeltmesi gerekli.
- Puanlar çoğunlukla düşük (yalnız ortak kelime): sürüm grubu tutan aday az.
  Yüksek puan (237, 253) kaynak + grup ya da kaynak + çözünürlük tuttuğunda.
  Sıralama yalnız indirme sırasını belirler; asıl eleme M4'ün kabul testi.
- `videoHash` gönderen istemcide `m: h` (hash eşleşmesi) ölçülmedi; gerçek
  oynatmada `hash_matches` alanından okunacak.
