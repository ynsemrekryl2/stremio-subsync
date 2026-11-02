# Ücretsiz sunucu: Oracle Cloud Always Free

SubSync çok az kaynak kullanır; Oracle'ın ücretsiz ARM sunucusu yeter.
Herhangi bir Ubuntu 22.04+/Debian 12+ sunucu da olur (Hetzner, Contabo, …):
o zaman yalnız 2. adımdaki portları kendi sağlayıcının panelinden aç.

## 1. Sunucuyu aç

Oracle konsolunda **Compute → Instances → Create instance**:

- **Image:** Canonical Ubuntu 24.04
- **Shape:** VM.Standard.A1.Flex (ARM), 1 OCPU / 6 GB yeter
  (ücretsiz sınır 4 OCPU / 24 GB)
- **SSH keys:** kendi public key'ini ekle (ya da Oracle'ın verdiğini indir)
- "Out of capacity" hatası alırsan başka bir Availability Domain seç ya da
  biraz sonra yeniden dene.

Açılınca sunucunun **Public IP**'sini not al.

## 2. 80 ve 443'ü aç

Let's Encrypt sertifikası için dışarıdan 80 ve 443'e ulaşılabilmeli.
Sunucunun sayfasında **Subnet → Security Lists → Default Security List →
Add Ingress Rules**:

| Source CIDR | IP Protocol | Destination Port |
|---|---|---|
| `0.0.0.0/0` | TCP | `80` |
| `0.0.0.0/0` | TCP | `443` |

Sunucunun kendi güvenlik duvarını kurulum betiği açar.

## 3. Kur

```bash
ssh ubuntu@<public-ip>
```

```bash
curl -fsSL https://raw.githubusercontent.com/ynsemrekryl2/stremio-subsync/main/install.sh | sudo bash
```

Sonu README'deki gibi: çıkan configure linkini aç.

## Bilinen risk

Oracle, Always Free hesaplarda 7 gün boyunca boşta kalan sunucuları geri
alabiliyor; SubSync çok az CPU kullandığı için bu risk var. Hesabı
Pay-As-You-Go'ya yükseltmek bunu kaldırır (ücretsiz sınırlar içinde
kalındıkça ücret çıkmaz, ama kart gerekir). Sunucu gitse bile configure
sayfasından indirdiğin yedekle yenisi dakikalar içinde aynı hale gelir.
