# SubSync — senkronu doğrulanmış altyazı (Stremio eklentisi)

Stremio'da oynattığın dosyaya **tam oturan** Türkçe (ya da seçtiğin dilde)
altyazıyı bulur:

- Dosyanın **içindeki İngilizce altyazının zamanlarını** cetvel olarak okur.
  Filmi indirmez; dosyanın yalnız içindekiler tablosunu okur.
- OpenSubtitles, SubDL, AltyaziDB, SubSource ve Wyzie'deki altyazıları bu
  cetvele kaydırır; filmin başında, ortasında ve sonunda tutanları sunar.
  Tutmayanı hiç göstermez.
- Hiçbiri uymazsa, dosyaya uyan İngilizce altyazıyı kendi OpenRouter
  anahtarınla çevirir (isteğe bağlı, film başına birkaç sent).

Dosyayı bulmak için **AIOStreams** kullanır (Real-Debrid, TorBox vb. onun
arkasında).

## Gerekenler

- Stremio ve bir **AIOStreams** kurulumu: onun `manifest.json` linki.
- Bir **Ubuntu ya da Debian sunucu**, 80 ve 443 portları açık. Ücretsiz yol:
  [Oracle Always Free](docs/oracle.md).
- İsteğe bağlı anahtarlar (SubDL, AltyaziDB, SubSource, Wyzie ücretsiz;
  OpenRouter ücretli). Hepsi kurulumdan sonra sayfadan girilir.

## Kurulum (5 adım)

1. **Sunucu aç.** Oracle için adım adım: [docs/oracle.md](docs/oracle.md).
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

Aynı sayfa bir panel de: her isteğin sonucu, kaynakların durumu, süren
çeviriler ve maliyeti, cache temizleme, yeniden başlatma, çeviri modeli.
Stremio'da eklentinin **Configure** düğmesi de bu sayfayı açar.

## Bir şey olursa

| Durum | Ne yap |
|---|---|
| Linki kaybettim | Sunucuda `subsync link` |
| Altyazı gelmiyor | Panelde "Son istekler" nedenini yazar; sunucuda `subsync` durumu gösterir |
| Bir anahtar değişti / bitti | Configure sayfasında yenisini yapıştır, **Dene**, **Kaydet** |
| Yeni sürüme geçmek | Sunucuda `subsync update` |
| Sunucu silindi, yenisini kurdum | Kurulum satırı, sonra configure sayfasında **Yedekten yükle**. IP aynıysa Stremio'da bir şey yapma; değiştiyse yeni linkle Stremio'ya yeniden kur |
| HTTPS açılmıyor | Sağlayıcının panelinde 80 ve 443 kapalıdır ([Oracle](docs/oracle.md#2-80-ve-443ü-aç)); sonra `sudo systemctl restart caddy` |
| Kaldırmak | `subsync uninstall` (token ve anahtarlar kalır; `--purge` hepsini siler) |

Sunucu komutları: `subsync` (durum) · `link` · `update` · `logs` · `restart`
· `backup` · `restore <dosya>` · `uninstall`.

## Daha fazlası

- [Ayarlar ve sunucudaki dosyalar](docs/config.md)
- [Geliştirme: Mac'te çalıştırma, kaynaktan deploy, testler](docs/development.md)
- [Nasıl çalışır](docs/plan.md) · [Kararlar](docs/decisions.md) · [İş listesi](TASKS.md)

## Lisans

[GPL-3.0 ya da sonraki sürümleri](LICENSE): kullanmak, değiştirmek ve
dağıtmak serbest; değiştirilmiş sürümü dağıtan kaynağını da açar.
Repoda kimsenin anahtarı yok: her kurulum kendi AIOStreams linkini ve
anahtarlarını configure sayfasından girer.
