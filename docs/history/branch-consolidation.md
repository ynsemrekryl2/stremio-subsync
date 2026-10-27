# Dal ve iş numarası birleştirmesi — SS-027

2026-10-27. Güncel çalışma çizgisi `main`; yeni işler buradaki `TASKS.md`
üzerinden numaralanır. Önceki commit mesajları tarihsel kayıttır; içlerindeki
numaralar aşağıdaki dal bağlamıyla okunur, commit geçmişi yeniden yazılmaz.

## Kodun birleştirilmesi

| Kaynak dal / son commit | Karar |
|---|---|
| `claude/m4-91d8d8` / `360a07b` | M2–M5 temeli, sonuç nedeni menüsü, kalıcı/geçici hata cache'i ve bütün İngilizce izlerden kabul edilenleri sunma düzeltmesi `main`'e alındı. |
| `claude/embedded-subtitle-translation-9c857d` / `6895893` | M4 dalıyla gerçek bir üç yönlü merge: blok probe'u, Cues konumları, çoklu Range denemesi ve çeviri planı alındı. `src/addon.rs` çakışması M5 akışını koruyup `block_probe` bayrağını bu akıştan geçirerek çözüldü. |
| `claude/devam-edelim-2bac6b` / `a1fee52` | Ek M2 ölçümü `docs/spikes/m2-ruler.md`'ye, Cues-only kararı plana/kararlara aktarıldı. Ana örneklemle çakışabileceği belirtildi; dosya sayıları toplanmadı. |
| `claude/m3e-basha-a4d3a4` / `9d284a7` | API alanları ve 6 başlık / 15 indirme ölçümü `docs/spikes/m3-sources-legacy.md`'de saklandı. İkinci, uyumsuz kaynak/SRT/sıralama uygulaması yerine M5'e bağlı, SubDL ve metni yeniden yazmayı destekleyen kod korundu. |
| `claude/devam-edelim-2bebd5` / `ff4df67` | Başlangıç `main` commit'i; ek değişiklik yok. |
| `claude/real-debrid-vs-torbox-c0b02b` / `88422d5` | Zaten `main` atası; ek değişiklik yok. |

Paralel M3 uygulamasının hash extra'sı / hash öncelikli sıralaması ve farklı
puanları güncel koda eklenmedi. Gerçek hash eşleşmesi o dalda da ölçülmemişti;
bu toparlama oturumu yeni kaynak davranışı veya eşik değişikliği yapmadı.
CD parçalarını isimden peşinen eleme yerine mevcut hizalamanın kapsama
kontrolü korunur; parçaları birleştirme SS-019 olarak açıktır.
İki eski paralel dalın yararlı kayıtları aktarıldıktan sonra eski kodları
yerine geçerli ağaç korunarak merge ataları kaydedilir (`ours` stratejisi).
Böylece eski commit'lere erişim kaybolmaz, bu kodlar tekrar merge adayı olmaz.

## Tekil iş numaraları

| Tarihsel dal bağlamı | Eski numara / anlamı | Güncel numara |
|---|---|---|
| `m3e-basha-a4d3a4` | SS-011 OpenSubtitles biçim ölçümü | SS-016 |
| `m3e-basha-a4d3a4` | SS-012 OpenSubtitles istemcisi | SS-016 |
| `m3e-basha-a4d3a4` | SS-013 sıralama / CD elemesi | SS-018; birleştirme SS-019 |
| `m3e-basha-a4d3a4` | SS-014 SRT zaman okuma | SS-015 |
| `m3e-basha-a4d3a4` | SS-015 SubDL | SS-017 |
| `m3e-basha-a4d3a4` | SS-016 aday toplama probe'u | SS-016 |
| `m4-91d8d8` | SS-025 PGS zaman cetveli | SS-025 (korundu) |
| `embedded-subtitle-translation-9c857d` | SS-025 hedefli metin blokları ölçümü | SS-028 |
| `embedded-subtitle-translation-9c857d` | SS-026 çeviri yedeği | SS-026 (korundu; SS-028'e bağlı) |
| `embedded-subtitle-translation-9c857d` | SS-027 dal/iş toparlama | SS-027 (bu iş) |

SS-011…014'ün güncel anlamları M4 hizalama işleridir. SS-009/010 iki M2
dalında da aynı işi tanımlar. Güncel iş listesinde her numara yalnız bir
iş satırında tanımlanır. Yeni numara SS-029'dan başlar.

## Eski worktree'ler ve yerel veriler

Altı eski dal `codex/archive/<eski-dalın-son-parçası>` adına taşınır.
Kaynak commit'leri `main` geçmişinde de korunur. Yeni geliştirme kökteki
`main` üzerinde yapılır.

Altı worktree'nin tümü açık Claude süreçlerinin çalışma dizini olduğu için
dizinler silinmedi; süreçler sonlandırılmadı. Oturumlar kapandıktan sonra
temiz durumları yeniden kontrol edilerek kaldırılabilir. Bu, eski dalların
yeniden güncel geliştirme çizgisi olduğu anlamına gelmez.

M4 worktree'sindeki **7 cache dosyası ve 14 kalibrasyon dump'ı** hem kökteki
`cache/` ve `dumps/` dizinlerine hem `.local/branch-backups/m4-91d8d8/`
altına kopyalandı; kopyalar SHA-256 ile doğrulandı. Yerel manifest
`.local/branch-backups/manifest.json` içindedir. Diğer worktree'lerde derleme
çıktısı dışında Git dışı veri bulunmadı. Bu dizinler Git'e eklenmez.

## Doğrulama

- `cargo fmt --check`
- `cargo test --offline --locked` (95 test)
- `cargo clippy --offline --locked --all-targets -- -D warnings`
- Birleştirme testi: blok okumaları hiç bitmese bile Stremio isteği
  “aday yok” yanıtını verir ve cache'ten tekrar sunar; arka plan blok probe'u
  gerçekten tetiklenir. Fake kaynaklar kullanılır, sağlayıcı ağı çağrılmaz.
- `TASKS.md`: SS-001…SS-028 eksiksiz ve tekil; yerel doküman bağlantıları ve
  Git çakışma işaretleri kontrol edilir.

Gerçek sağlayıcı veya Stremio testi bu oturumda tekrarlanmadı. M2/M3/M5'in
önceki ölçümleri korundu; blok okuma ve çeviri tamamlanmış sayılmadı.
