# SubSync — senkronu doğrulanmış altyazı (Stremio eklentisi)

Stremio'da oynattığın dosyaya **tam oturan** Türkçe (ya da seçtiğin dilde)
altyazıyı bulur. Diğer altyazı eklentileri bulduklarını listeler, hangisinin
oturduğunu tek tek deneyerek anlarsın; SubSync bu denemeyi senin yerine
yapar:

- Dosyanın **içindeki İngilizce altyazının zamanlarını** cetvel olarak okur.
  Filmi indirmez; dosyanın yalnız içindekiler tablosunu okur.
- Altı kaynaktaki altyazıları (SRT, ASS, VTT; ZIP ya da RAR içinde de) bu
  cetvele kaydırır, kare hızı farkını düzeltir; filmin başında, ortasında ve
  sonunda tutanları sunar. Tutmayanı hiç göstermez.
- Hiçbiri uymazsa, dosyaya uyan İngilizce altyazıyı kendi OpenRouter
  anahtarınla çevirir (isteğe bağlı, film başına birkaç sent).

Dosyayı bulmak için **AIOStreams** kullanır (Real-Debrid, TorBox vb. onun
arkasında).

## Kaynaklar

| Kaynak | Anahtar | Nereden |
|---|---|---|
| OpenSubtitles | Gerekmez | Stremio'nun anahtarsız OpenSubtitles v3 eklentisi |
| The Subtitle Database | Gerekmez | |
| SubDL | Ücretsiz | [subdl.com](https://subdl.com) hesabının API anahtarı |
| AltyaziDB | Ücretsiz | [altyazidb.com](https://altyazidb.com) üyeliği; yalnız Türkçe ve İngilizce |
| SubSource | Ücretsiz | [subsource.net](https://subsource.net) hesap panelinin API sekmesi |
| Wyzie Subs | Ücretsiz | [store.wyzie.io/redeem](https://store.wyzie.io/redeem), e-posta doğrulamasıyla |
| OpenRouter (çeviri) | Ücretli | [openrouter.ai](https://openrouter.ai); yoksa çeviri kapalı |

Anahtarı girilmeyen kaynak aranmaz; girilenlerin hepsi aynı anda aranır.

## Gerekenler

SubSync, diğer self-hosted Stremio eklentileri gibi kendi sunucunda çalışır:
Stremio ona internetten HTTPS ile bağlanır, anahtarların yalnız o sunucuda
durur.

- Stremio ve bir **AIOStreams** kurulumu: onun `manifest.json` linki.
- Bir **sunucu**, x86_64 ya da ARM64: herhangi bir VPS ya da evdeki bir
  makine. Tek komutlu kurulum için Debian 12+ ya da Ubuntu 22.04+; Docker
  varsa herhangi bir Linux.
- Dışarıdan erişim: **80 ve 443 portları açık** (bulut sağlayıcının
  firewall / security group ayarında, evdeyse modemde port yönlendirmeyle).
- Alan adı tek komutlu kurulumda isteğe bağlı: yoksa sunucunun IP'sinden
  bir `<ip>.sslip.io` adresi ve onun HTTPS sertifikası kendiliğinden alınır.

## Kurulum (tek komut)

1. **Sunucuya bağlan ve kur:**

   ```bash
   curl -fsSL https://raw.githubusercontent.com/ynsemrekryl2/stremio-subsync/main/install.sh | sudo bash
   ```

   Hiçbir şey sormaz: programı indirir, HTTPS'i (Caddy) ve servisi kurar,
   sunucunun kendi güvenlik duvarında 80 ve 443'ü açar, sonunda **configure
   sayfasının linkini** verir. Kendi alan adın için `sudo` ile `bash`
   arasına `SUBSYNC_DOMAIN=subs.example.com` ekle.
2. **Configure sayfasını aç.** AIOStreams linkini (zorunlu) ve istediğin
   anahtarları gir; her birini **Dene** ile sına, **Kaydet**.
3. **Stremio'ya kur** düğmesine bas. Eklenti Stremio hesabına eklenir; aynı
   hesaptaki TV'lerde de görünür.
4. **Yedeği indir** ve şifre yöneticine koy. Sunucu giderse yenisinde
   "Yedekten yükle" ile her şey geri gelir.

Configure sayfasının linki kurulumun şifresidir; kimseyle paylaşma.

## Docker ile kurulum

Sunucunda zaten Docker varsa (ev sunucusu, NAS, başka eklentiler…) betik
yerine bunu kullan. İmaj x86_64 ve ARM64 için:
`ghcr.io/ynsemrekryl2/stremio-subsync`.

1. Boş bir klasörde [`compose.yaml`](compose.yaml)'ı indir ve token'ı
   üret:

   ```bash
   curl -fsSLO https://raw.githubusercontent.com/ynsemrekryl2/stremio-subsync/main/compose.yaml
   ```

   ```bash
   echo "SUBSYNC_TOKEN=$(openssl rand -hex 32)" > .env
   ```

2. **HTTPS'i seç.** Stremio eklentiyi yalnız HTTPS ile kurar:
   - **Kendi reverse proxy'n varsa** (Nginx Proxy Manager, Traefik,
     Caddy…): onu `127.0.0.1:7001`'e, proxy bir Docker konteyneriyse
     `subsync:7001`'e yönlendir (aynı Docker ağına bağlayarak).
   - **Yoksa** bir alan adını sunucuya yönlendir ve `.env`'e şu iki satırı
     ekle; Caddy sertifikayı kendisi alır (80 ve 443 açık olmalı):

     ```
     COMPOSE_PROFILES=https
     SUBSYNC_DOMAIN=subs.example.com
     ```

3. **Başlat:**

   ```bash
   docker compose up -d
   ```

4. Configure sayfası `https://<adres>/<token>/configure`; token `.env`'de.
   Sonrası yukarıdaki 2–4. adımlarla aynı.

Güncellemek için `docker compose pull && docker compose up -d`, kayıtlar için
`docker compose logs -f subsync`. Anahtarlar, ayarlar ve cache
`subsync-data` volume'ünde kalır.

Docker'sız, kendi proxy'nin arkasında çalıştırmak istersen
[Releases](https://github.com/ynsemrekryl2/stremio-subsync/releases/latest)
sayfasındaki `stremio-subsync-<x86_64|aarch64>-linux` dosyasını
`SUBSYNC_TOKEN` (en az 32 karakter) ile çalıştırıp proxy'yi `127.0.0.1:7001`'e
yönlendirmen yeter.

## Kullanım

Stremio'da bir video aç, altyazı menüsünde dilinin altındaki **SubSync**
girdilerini seç. İlk açılışta yanıt birkaç saniye sürebilir (en çok 15 sn),
sonrakiler hazır gelir.

Altyazı bulunamazsa menüde bir **SubSync Hata** girdisi çıkar; seçince
nedenini ve ne yapacağını yazar. Sık nedenler:

- **Dosyada İngilizce altyazı yok:** cetvel yok. Aynı filmin İngilizce
  altyazılı başka bir sürümünü (genelde MKV) seç.
- **Dosya türü desteklenmiyor:** MKV ve MP4 okunur; AVI, MPEG-TS ve parçalı
  MP4 okunmaz.
- **Hiçbiri tutmadı:** o sürüme uyan altyazı yok. Çeviri açıksa hazırlanıyor
  demektir; birkaç dakika sonra videoyu yeniden aç.

Anime kataloglarının (Kitsu) bölümleri de aranır: Kitsu numarası IMDb
numarasına, sezonuna ve bölümüne çevrilir (Fribb'in anime-lists listesiyle,
haftada bir güncellenir). Listede IMDb karşılığı olmayan başlıklar aranmaz.
Eski bir kurulumda Kitsu bölümlerinde SubSync görünmüyorsa eklentiyi
Stremio'ya yeniden kur.

Aynı sayfa bir panel de, üç sekmeli: **Durum** (son isteklerin özeti,
kaynakların durumu, süren çeviriler ve maliyeti, her isteğin sonucu),
**Kurulum** (bağlantılar, anahtarlar, Stremio'ya kurma, yedek) ve
**Ayarlar** (çeviri modeli, cache temizleme, yeniden başlatma). Stremio'da
eklentinin **Configure** düğmesi bu sayfayı Kurulum sekmesinde açar.

## Bir şey olursa

| Durum | Ne yap |
|---|---|
| Linki kaybettim | Sunucuda `subsync link` (Docker: token `.env`'de) |
| Altyazı gelmiyor | Menüdeki **SubSync Hata** girdisi ve paneldeki "Son istekler" nedenini yazar; sunucuda `subsync` durumu gösterir |
| Bir anahtar değişti / bitti | Configure sayfasında yenisini yapıştır, **Dene**, **Kaydet** |
| Yeni sürüme geçmek | Sunucuda `subsync update` (Docker: `docker compose pull && docker compose up -d`) |
| Sunucu silindi, yenisini kurdum | Kurulum satırı, sonra configure sayfasında **Yedekten yükle**. Adres aynıysa Stremio'da bir şey yapma; değiştiyse yeni linkle Stremio'ya yeniden kur |
| HTTPS açılmıyor | Sunucunun dışındaki güvenlik duvarı (sağlayıcının paneli ya da modem) 80 ve 443'ü kapalı tutuyordur; açınca `sudo systemctl restart caddy` |
| Kaldırmak | `subsync uninstall` (token ve anahtarlar kalır; `--purge` hepsini siler) |

Sunucu komutları: `subsync` (durum) · `link` · `update` · `logs` · `restart`
· `backup` · `restore <dosya>` · `uninstall`.

## Geliştirme

Rust; `cargo run` için `SUBSYNC_TOKEN` (en az 32 karakter) yeter, gerisi
`http://127.0.0.1:7001/<token>/configure` sayfasından girilir. Testler ağ
kullanmaz:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

`v*` etiketi GitHub Actions'ta x86_64 ve ARM64 Linux sürümünü yayınlar;
sunucular `subsync update` ile alır.

## Lisans

[GPL-3.0 ya da sonraki sürümleri](LICENSE): kullanmak, değiştirmek ve
dağıtmak serbest; değiştirilmiş sürümü dağıtan kaynağını da açar.
Repoda kimsenin anahtarı yok: her kurulum kendi AIOStreams linkini ve
anahtarlarını configure sayfasından girer.
