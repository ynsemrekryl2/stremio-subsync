# Plan

## Akış

```
Stremio ──GET /{token}/subtitles/{type}/{id}/{extra}──▶ add-on
   extra: filename, videoSize, (videoHash)

1. Dosyayı tanı     AIOStreams /stream/{type}/{id}.json → filename + boyutu
                    tutan stream → oynatma URL'si (arkada Real-Debrid veya TorBox)
2. Cetveli oku      HTTP Range: EBML başlığı → SeekHead → Tracks + Cues →
                    İngilizce metin izinin bloklarını tek tek oku → cue zamanları
3. Adayları topla   OpenSubtitles v3 (anahtarsız) + SubDL (ücretsiz anahtar);
                    sürüm adı dosyaya en çok benzeyen önce
4. Hizala ve test   satır örtüşmesi (10 ms ızgara, tam hesap) her fps adayı için;
                    baş / orta / son aynı kaydırmada tutmazsa reddet
5. Teslim           kabul edilenleri SRT olarak sun; sonucu dosya kimliğiyle cache'le
```

Hiçbir aşamada film indirilmez. Okuma bütçesi dosya başına sınırlıdır; bütçe
aşılırsa o dosya için senkronlu altyazı üretilmez.

## Aşamalar

| # | Ad | Bitince ne doğru olur |
|---|---|---|
| **M0** | İskelet | Token'lı add-on sunucusu Stremio'ya kurulabiliyor, boş yanıt dönüyor. |
| **M1** | Keşif | Üç soru gerçek cihaz/dosyayla cevaplanmış (aşağıda). Cevaplar planı değiştirebilir. |
| **M2** | Cetvel | Gerçek bir MKV'den, filmi indirmeden İngilizce altyazı zamanları çıkıyor. |
| **M3** | Kaynaklar | Bir IMDb id + dil için aday altyazılar indirilip sıralanıyor. |
| **M4** | Hizalama | Kaydırma + fps düzeltmesi + kabul testi; etiketli test setinde yanlış kabul yok. |
| **M5** | Teslim | Uçtan uca çalışıyor; cache; VPS'te HTTPS ile kurulu. |
| **M6** | Ekstralar | İsteğe bağlı: tek replik ince ayarı, AI çeviri yedeği, ikinci/üçüncü dil. |

## M1 keşif soruları

| # | Soru | Neden önemli |
|---|---|---|
| **K1** | Stremio masaüstü, Android TV ve Apple TV altyazı isteğinde `filename`, `videoSize`, `videoHash` alanlarından hangilerini gönderiyor? | Dosya tanıma bunlara dayanıyor. Apple TV Lite'ta hash gelmeyebilir. |
| **K2** | AIOStreams'in stream listesinde bu alanlarla tek bir stream'i kesin eşleyebiliyor muyuz? URL'yi çağırmak debrid tarafında ne tetikliyor? | Real-Debrid ve TorBox'a ayrı API yazmadan ikisini de kapsamanın yolu bu. |
| **K3** | Real-Debrid ve TorBox linkleri Range isteğini destekliyor mu, ve gerçek MKV'lerin Cues'u altyazı izini indeksliyor mu? (en az 10 dosya) | Indekslemiyorsa "birkaç MB okuma" mümkün değil; bu dosyalar desteklenmez. |

## Kapsam dışı (şimdilik)

Ses analizi · MP4/tx3g ve görüntü tabanlı (PGS) altyazılar · çok kullanıcı ·
hesap sistemi · web ayar sayfası · AIOStreams olmadan doğrudan debrid API'leri.
