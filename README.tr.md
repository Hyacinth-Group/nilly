# discord-bot

Sohbet hafızası ve zamanla değişen bir personası olan, kendi makinenizde çalıştırabileceğiniz bir Discord botu.

[English](README.md) · [Kurulum rehberi](docs/setup-guide.md) · [Sürümler](https://github.com/krxi/discord-bot/releases) · [Değişiklikler](CHANGELOG.md)

## Nasıl çalışır?

discord-bot bir Discord sunucusunda çalışır ve slash komutu beklemeden sohbete katılabilir. Arka plandaki model çağrıları kişiler, konular ve botun önceki cevapları hakkında notlar yazar; sonraki cevaplar bu notları sabit bir kişilik promptuyla birlikte kullanır. Uyku düzeni ve gelişim evreleri bazı davranışlarını belirler. Bot Rust ile yazılmıştır, hafızasını yerel olarak saklar ve model çağrıları için sizin OpenRouter veya Mistral anahtarınızı kullanır. Sohbet bağlamı makinenizden dışarı gönderilir; arka plan işleri de API kullanımınıza dahildir.

İnsan bir sunucu üyesi değil, yapay zeka botudur. Varsayılan persona kimlik sorularına dürüstçe cevap verir. Henüz yayımlanmış bir Discord kaydı veya uzun dönem kullanım çalışması yok; kayıt tarifi, kişilik gelişiminin kanıtı değildir.

## Veri akışı ve gizlilik

> **Davet etmeden önce üyelere hangi kanalları okuyabileceğini söyleyin.** Botu başlatmadan önce tarayıcı kurulumunda `CHANNELS` ayarlayın. Boş `GUILD_ID` / `CHANNELS`, yalnız bota hitap edilen sohbetler değil, botun erişebildiği tüm sunucular / kanallar demektir.

| Hedef | Gönderilen veya saklanan veri |
|---|---|
| Makineniz | `durum/hafiza.redb` içinde kişi/konu notları, özetler, kanal geçmişi ve panel kimlik doğrulama kayıtları; `durum/arsiv/` içinde taşan ham metin ve oluşturduğunuz yedekler. Özel izinli `.env` içinde bot/model anahtarları (Docker: `config/.env`). |
| Model hizmeti | Cevaplar ve arka plan analizi için seçilmiş sohbet metni ve kayıtlı notlar. OpenRouter kullanıldığında isteği karşılayan model sağlayıcısı da veriyi işler. Ekli görseller dahil edilebilir: görsel analizi varsayılan olarak açıktır (`IMAGE_ANALYSIS=off` kapatır). |
| Firecrawl, ayarlanmışsa | Web okuma/arama için sayfa adresleri ve arama sorguları. |

İlk taramada, kanal kısıtları ve Discord izinleri dahilinde erişilebilir metin kanallarının **14 güne kadar** geçmişini okur. Uyurken de mesajları gözlemler. Yerel depolama, sohbet bittikten sonra özet ve arşivleri saklayabilir. **Üye bazında seçici silme desteklenmez; `/reset` veri silme komutu değildir.** Yedekler dahil ayrıntılar için [depolama ve gizlilik](docs/setup-guide.md#privacy). Kendi makinenizde çalışması, tüm işlemenin yerel olduğu anlamına gelmez.

## Zamanla ne değişiyor?

- **Cevapların arkasındaki notlar.** Arka plan analizi grup profilini, huy notlarını ve sohbet sonrası düzeltmeleri günceller. Bu notlar sonraki cevaplara bağlam olur; model ağırlıkları değişmez.
- **Hatırladığı bilgiler.** Kişi ve konu kayıtları yerel redb veritabanında saklanır, yeniden başlatınca kaybolmaz. Getirilen hafıza sınırlıdır; eski malzeme özetlenir. Arşiv insanların incelemesi içindir, bot burada arama yapmaz.
- **Sohbete katıldığı anlar.** Normal mesajlar bir isteklilik kontrolünden geçer. Bot uyanıkken etiketlenmesi veya devam eden bir konuşma bu kontrolü atlayabilir. Cevap verebilir, tepki bırakabilir veya susabilir.

Temel persona ve gelişim eşikleri kaynak kodda tanımlıdır. Bu bir prompt ve hafıza deneyi; yararlı kişilik değişimini göstermek için zaman içinde gerçekleşmiş gerçek sohbetler gerekir.

## Maliyet

Henüz ölçülmüş bir çalıştırma maliyeti yayımlamadık. Cevaplar ve arka plan analizi model anahtarınızı kullanır. Sağlayıcı harcama limitiyle başlayın ve kullanım panelini kontrol edin.

Kurulumun varsayılan modelleri **ücretlidir**. 11 Ekim 2026'da canlı OpenRouter listesinde doğrulanan ücretsiz bir örnek: `google/gemma-4-31b-it:free`. Erişilebilirlik ve limitler değişebilir; bu model botla uçtan uca test edilmedi. Arka plan çağrıları da ücretsiz limiti tüketebilir. Barındırma ayrıca ücretli olabilir.

## Hızlı başlangıç — Docker

Discord bot tokenı, model API anahtarı ve çalışan [Docker](https://docs.docker.com/get-started/get-docker/) gerekir. Kurulum, Terminal veya PowerShell'e komut kopyalamayı gerektirir. [Adım adım rehber](docs/setup-guide.md), Discord ayarlarını ve Windows komutlarını açıklar.

[Developer Portal](https://discord.com/developers/applications) → **Bot → Privileged Gateway Intents** altında **Message Content Intent** ve **Server Members Intent** açın; Presence Intent açmayın. Anahtarları gizli tutun.

macOS/Linux için:

```sh
mkdir mybot && cd mybot
curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
mkdir -p config photos
```

Linux'ta yapılandırma dizinini yalnız konteynerin UID 10001 kullanıcısının yazabileceği şekilde ayarlayın:

```sh
sudo chown 10001:10001 config
sudo chmod 700 config
```

Docker Desktop'ta sahiplik farklı yönetilir; [kurulum rehberine](docs/setup-guide.md) bakın. Başlangıçta `.env` veya CLI sihirbazı gerekmez. Paneli başlatın:

```sh
docker compose up -d
docker compose logs bot
```

Loglarda yazdırılan tek kullanımlık `http://localhost:8080/setup#token=…` bağlantısını açın; **15 dakika** sonra geçersiz olur. Beş adımı izleyin: en az **12 karakterli** panel parolası belirle → Discord tokenını doğrula → sağlayıcı anahtarını doğrula → dil/saat dilimi, sahip, erişim kısıtları ve haberleri ayarla → davet et ve başlat. Kaydedilen panel parolası kurulum yarım kalsa da korunur; giriş yaparak devam edin. Kurulum botu süreç yeniden başlatılmadan çalıştırır. Anahtar testleri Discord uygulama kimliğini ve sağlayıcı kimlik doğrulamasını kontrol eder; **ücretli completion çağrısı yapmaz**, ayrıcalıklı intentleri, model uyumluluğunu veya gateway bağlantısını kanıtlamaz.

Discord'da **`/status`** (Türkçe için `BOT_LANG=tr` seçtiyseniz `/durum`) çalıştırarak doğrulayın, ardından botu etiketlemeyi deneyin. Normal sohbetlere cevap vermeyebilir; geceleri uyur. Docker'ı açık tutun; hatalar için `docker compose logs -f` kullanın.

Compose yalnız host loopback portu 8080'i açar; atomik ayar kaydı için gerekli olan yazılabilir `./config` **dizinini** ve salt okunur `./photos` dizinini bağlar; hafıza ve panel sahipliğini sabit adlı **`discord-bot-data`** volume'ünde tutar. [Güncellemeden önce yedekleyin](docs/setup-guide.md#back-up-the-memory); `docker compose pull && docker compose up -d` ile güncelleyin. Mevcut kurulumlarda [`env_file` kaldırılmadan önce `.env` özel izinli `config/.env` konumuna taşınmalı ve eski hafıza volume'ü korunmalıdır](docs/setup-guide.md#existing-compose-installations)—başlatma sorununu çözmek için eski veriyi atmayın.

Başka yol mu tercih ediyorsunuz? [Hazır binary (terminal gerekir)](docs/setup-guide.md#download-a-release) · [Kaynaktan derleme](docs/setup-guide.md#build-from-source). Aynı tarayıcı kurulumunu açmak için binary'yi `.env` olmadan çalıştırın. Tarayıcısız kurulum için `discord-bot init` kullanılabilir; paneli kapatmak için `PANEL_ENABLED=off` ayarlayın.

## Panel sınırları ve parola kurtarma

Gömülü panel kurulum/onarım, çalışma ve gateway durumu, konuşma bazında ruh hali, doğrulanan ve anahtarları geri göstermeyen ayarlar, canlı loglar, hesap yönetimi ve yardım sunar. Hafıza/kişilik editörleri, eylem düğmeleri ve ayrı kullanım sayfası ortak çekirdek API'leri tamamlanana kadar **kapalıdır**. Genel bakış süreç içi çağrı/token sayaçlarını gösterir; ölçülmüş çalıştırma maliyeti göstermez.

Kaydedilen ayarlar yeniden başlatma gerektirir (`docker compose restart bot`); hafızadaki `model.md` model tercihi öncelikli olmaya devam eder. Süreç başlangıcında verilen ortam değişkenleri yapılandırma dosyasına göre önceliklidir ve panelde salt okunurdur. Canlı loglar sınırlı, hassas değerleri maskelenmiş bir yakın geçmiş tamponudur; **kalıcı denetim kaydı değildir** ve yine de özel sohbet metni içerebilir.

Genel internete açık port yerine localhost veya SSH tüneli/VPN kullanın. HTTPS ters proxy için açıkça `PANEL_PUBLIC_URL` ayarlanmalı ve Host/Origin başlıkları korunmalıdır; forwarded başlıkları güvenilir kaynak sayılmaz. Ayrıntılar [kurulum rehberinde](docs/setup-guide.md).

Parola kurtarmak için botu durdurun; normal çalışma dizininde, aynı `durum/hafiza.redb` kullanılarak `discord-bot panel reset-password` çalıştırın. Bir kez yazdırılan yeni parolayı kaydedip yeniden başlatın; mevcut sahip kimliği korunur. Docker için:

```sh
docker compose stop bot
docker compose run --rm bot panel reset-password
docker compose up -d
```

## Sınırlar ve başvuru

- Hafıza getirmenin karakter bütçesi vardır; saklanan malzeme kusursuz hatırlama demek değildir.
- Bazı kalıcı anahtarlar ve kalan gün adı/ruh hali mantığı Türkçedir. Seyahat Türk tatil takvimini kullanır ve İngilizce modda kapalıdır.
- `STEALTH`, **azaltılmış bot arayüzü** demektir; insan gibi görünmek değil. Slash etkileşimlerini/arayüz embedlerini kapatır, canlı düzenleme yerine tamamlanmış cevap gönderir. `/status` doğrulaması için kapalı bırakın.
- Promptlar güvenlik sınırı değildir. Kanal izinleri ve sağlayıcı harcama kontrollerini kullanın.

[Mimari ve ajanlar](docs/architecture.md) · [Hafıza biçimleri](docs/state-files.md) · [Yapılandırma](docs/configuration.md) · [Komutlar](docs/commands.md) · [Dokümantasyon dizini](docs/README.md)

## Katkı ve lisans

Fork alın, dal oluşturun ve pull request açın. Rust bilmeden de dokümanlar, çeviriler ve tekrarlanabilir hata raporlarıyla katkı verebilirsiniz. Bkz. [CONTRIBUTING.md](CONTRIBUTING.md).

MIT — bkz. [LICENSE](LICENSE).
