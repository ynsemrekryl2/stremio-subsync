# stremio-subsync — Çalışma düzeni

Hafif düzen: tek geliştirici, kişisel kullanım. Oturum başında `TASKS.md`,
gerekirse `docs/plan.md` ve `docs/decisions.md` okunur.

## Dil

- **Türkçe:** dokümanlar, plan, task açıklamaları, kullanıcıyla iletişim.
- **İngilizce:** kod, isimler, dosya adları, yorumlar, test adları, commit mesajları.

## Kurallar

1. Her iş `TASKS.md`'de bir satırla başlar (`SS-###`). Yol üstünde görülen
   başka iş yeni satır olur, mevcut işe eklenmez.
2. Bir iş, kanıtı olmadan bitmiş sayılmaz: logic → test, spike → ölçüm notu
   (`docs/spikes/`), deploy → çalışan link kontrolü.
3. Büyük kararlar `docs/decisions.md`'ye tarih ve gerekçeyle yazılır.
4. Testler gerçek provider kotası, debrid trafiği veya ağ kullanmaz; fake zorunlu.
5. Commit, testler yeşilken ve iş kendi başına anlamlıyken atılır.
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
