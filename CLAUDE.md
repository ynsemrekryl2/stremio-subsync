# stremio-subsync — Çalışma düzeni

Hafif düzen: tek geliştirici, kişisel kullanım. Oturum başında `TASKS.md`,
gerekirse `docs/plan.md` ve `docs/decisions.md` okunur.

## Dil

- **Türkçe:** dokümanlar, plan, task açıklamaları, kullanıcıyla iletişim.
- **İngilizce:** kod, isimler, dosya adları, yorumlar, test adları, commit mesajları.

## Kurallar

1. Güncel kod ve iş listesi `main` üzerindedir. Yeni işe başlamadan önce
   `git status`, `git worktree list` ve dalların durumunu kontrol et. SS
   numaraları yalnız `main`'deki `TASKS.md` üzerinden ayrılır; paralel dal
   kendi başına numara üretmez. Geçmiş numara eşlemeleri
   `docs/history/branch-consolidation.md` içindedir.
2. Her iş `TASKS.md`'de bir satırla başlar (`SS-###`). Yol üstünde görülen
   başka iş yeni satır olur, mevcut işe eklenmez.
3. Bir iş, kanıtı olmadan bitmiş sayılmaz: logic → test, spike → ölçüm notu
   (`docs/spikes/`), deploy → çalışan link kontrolü.
4. Büyük kararlar `docs/decisions.md`'ye tarih ve gerekçeyle yazılır.
5. Testler gerçek provider kotası, debrid trafiği veya ağ kullanmaz; fake zorunlu.
6. Commit, testler yeşilken ve iş kendi başına anlamlıyken atılır.
   Mesaj formatı: `feat(mkv): read subtitle cues (SS-004)`.

## Asla loglanmayacaklar

Kurulum token'ı · medya/stream URL'si · AIOStreams manifest linki · API key ·
dosya adı · IMDb id · altyazı metni. `Debug` implementasyonu bu alanları
basmaz; gerektiğinde yalnız "var/yok" bilgisi loglanır.

## Doğrulama

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
