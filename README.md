# SubSync — senkronu doğrulanmış altyazı (Stremio eklentisi)

Stremio'da oynattığın dosyaya **tam oturan** Türkçe (ya da seçtiğin dilde)
altyazıyı bulur:

- Dosyanın **içindeki İngilizce altyazının zamanlarını** cetvel olarak okur.
  Filmi indirmez; dosyanın yalnız içindekiler tablosunu okur.
- OpenSubtitles, The Subtitle Database, SubDL, AltyaziDB, SubSource ve
  Wyzie'deki altyazıları (SRT, ASS, VTT; ZIP ya da RAR içinde de) bu
  cetvele kaydırır; filmin başında, ortasında ve sonunda tutanları sunar.
  Tutmayanı hiç göstermez.
- Hiçbiri uymazsa, dosyaya uyan İngilizce altyazıyı kendi OpenRouter
  anahtarınla çevirir (isteğe bağlı, film başına birkaç sent).

Dosyayı bulmak için **AIOStreams** kullanır (Real-Debrid, TorBox vb. onun
arkasında).

## Gerekenler

- Stremio ve bir **AIOStreams** kurulumu: onun `manifest.json` linki.
- Bir **Ubuntu 22.04+ ya da Debian 12+ sunucu** (Intel ya da ARM), 80 ve 443
  portları dışarıya açık. Ücretsiz yol: Oracle Cloud Always Free.
- İsteğe bağlı anahtarlar (SubDL, AltyaziDB, SubSource, Wyzie ücretsiz;
  OpenRouter ücretli). Hepsi kurulumdan sonra sayfadan girilir.

## Kurulum (5 adım)

1. **Sunucu aç** ve 80, 443 portlarını dışarıya aç. Oracle Always Free'de:
   Compute → Create instance, Canonical Ubuntu 24.04, VM.Standard.A1.Flex;
   sonra sunucunun ağında Security List → Add Ingress Rules: kaynak
   `0.0.0.0/0`, TCP, port 80; aynısı 443 için.
2. **Sunucuya bağlan ve kur:**

   ```bash
   curl -fsSL https://raw.githubusercontent.com/ynsemrekryl2/stremio-subsync/main/install.sh | sudo bash
   ```

   Hiçbir şey sormaz: programı indirir, HTTPS'i ve servisi kurar, sonunda
   **configure sayfasının linkini** verir.
3. **Configure sayfasını aç.** AIOStreams linkini (zorunlu) ve istediğin
   anahtarları gir; her birini **Dene** ile sına, **Kaydet**.
4. **Stremio'ya kur** düğmesine bas. Eklenti Stremio hesabına eklenir; aynı
   hesaptaki TV'lerde de görünür.
5. **Yedeği indir** ve şifre yöneticine koy. Sunucu giderse yenisinde
   "Yedekten yükle" ile her şey geri gelir.

Configure sayfasının linki kurulumun şifresidir; kimseyle paylaşma.

## Kullanım

Stremio'da bir video aç, altyazı menüsünde **SubSync** girdilerini seç. İlk
açılışta yanıt birkaç saniye sürebilir (en çok 15 sn), sonrakiler hazır
gelir. Altyazı bulunamazsa menüde nedenini yazan bir girdi görünür; çeviri
hazırlanıyorsa birkaç dakika sonra videoyu yeniden aç.

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
| Linki kaybettim | Sunucuda `subsync link` |
| Altyazı gelmiyor | Panelde "Son istekler" nedenini yazar; sunucuda `subsync` durumu gösterir |
| Bir anahtar değişti / bitti | Configure sayfasında yenisini yapıştır, **Dene**, **Kaydet** |
| Yeni sürüme geçmek | Sunucuda `subsync update` |
| Sunucu silindi, yenisini kurdum | Kurulum satırı, sonra configure sayfasında **Yedekten yükle**. IP aynıysa Stremio'da bir şey yapma; değiştiyse yeni linkle Stremio'ya yeniden kur |
| HTTPS açılmıyor | Sağlayıcının panelinde 80 ve 443 kapalıdır (1. adım); açınca `sudo systemctl restart caddy` |
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

`v*` etiketi GitHub Actions'ta Intel ve ARM Linux sürümünü yayınlar;
sunucular `subsync update` ile alır.

## Lisans

[GPL-3.0 ya da sonraki sürümleri](LICENSE): kullanmak, değiştirmek ve
dağıtmak serbest; değiştirilmiş sürümü dağıtan kaynağını da açar.
Repoda kimsenin anahtarı yok: her kurulum kendi AIOStreams linkini ve
anahtarlarını configure sayfasından girer.
