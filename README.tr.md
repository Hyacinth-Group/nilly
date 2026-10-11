# discord-bot

Sohbet hafızası ve zamanla değişen bir personası olan, kendi makinenizde çalıştırabileceğiniz bir Discord botu.

[English](README.md) · [Kurulum rehberi](docs/setup-guide.md) · [Sürümler](https://github.com/krxi/discord-bot/releases) · [Değişiklikler](CHANGELOG.md)

## Nasıl çalışır?

discord-bot bir Discord sunucusunda çalışır ve slash komutu beklemeden sohbete katılabilir. Arka plandaki model çağrıları kişiler, konular ve botun önceki cevapları hakkında notlar yazar; sonraki cevaplar bu notları sabit bir kişilik promptuyla birlikte kullanır. Uyku düzeni ve gelişim evreleri bazı davranışlarını belirler. Bot Rust ile yazılmıştır, hafızasını yerel olarak saklar ve model çağrıları için sizin OpenRouter veya Mistral anahtarınızı kullanır. Sohbet bağlamı makinenizden dışarı gönderilir; arka plan işleri de API kullanımınıza dahildir.

İnsan bir sunucu üyesi değil, yapay zeka botudur. Varsayılan persona kimlik sorularına dürüstçe cevap verir. Henüz yayımlanmış bir Discord kaydı veya uzun dönem kullanım çalışması yok; kayıt tarifi, kişilik gelişiminin kanıtı değildir.

## Veri akışı ve gizlilik

> **Davet etmeden önce üyelere hangi kanalları okuyabileceğini söyleyin.** İlk çalıştırmadan önce `CHANNELS` ayarlayın. Boş `GUILD_ID` / `CHANNELS`, yalnız bota hitap edilen sohbetler değil, botun erişebildiği tüm sunucular / kanallar demektir.

| Hedef | Gönderilen veya saklanan veri |
|---|---|
| Makineniz | `durum/hafiza.redb` içinde kişi/konu notları, özetler ve kanal geçmişi; `durum/arsiv/` içinde taşan ham metin ve oluşturduğunuz yedekler. |
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

Sihirbazın varsayılan modelleri **ücretlidir**. 11 Ekim 2026'da canlı OpenRouter listesinde doğrulanan ücretsiz bir örnek: `google/gemma-4-31b-it:free`. Erişilebilirlik ve limitler değişebilir; bu model botla uçtan uca test edilmedi. Arka plan çağrıları da ücretsiz limiti tüketebilir. Barındırma ayrıca ücretli olabilir.

## Hızlı başlangıç — Docker

Discord bot tokenı, model API anahtarı ve çalışan [Docker](https://docs.docker.com/get-started/get-docker/) gerekir. Kurulum, Terminal veya PowerShell'e komut kopyalamayı gerektirir. [Adım adım rehber](docs/setup-guide.md), Discord ayarlarını ve Windows komutlarını açıklar.

[Developer Portal](https://discord.com/developers/applications) → **Bot → Privileged Gateway Intents** altında **Message Content Intent** ve **Server Members Intent** açın; Presence Intent açmayın. Anahtarları gizli tutun.

macOS/Linux için:

```sh
mkdir mybot && cd mybot
curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
docker run --rm -it --user "$(id -u):$(id -g)" -v "$PWD:/data" ghcr.io/krxi/discord-bot:latest init
```

Başlatmadan önce oluşturulan `.env` dosyasında `GUILD_ID` ve `CHANNELS` değerlerini hedef sunucu/kanallarla kısıtlayın. Sonra:

```sh
docker compose up -d
```

Sihirbazın davet bağlantısını açın, sunucunuzu seçin ve Discord'da **`/status`** (Türkçe için `BOT_LANG=tr` seçtiyseniz `/durum`) çalıştırarak doğrulayın. Ardından botu etiketlemeyi deneyin. Normal sohbetlere cevap vermeyebilir; geceleri uyur. Docker'ı açık tutun; hatalar için `docker compose logs -f` kullanın.

Kayıtlı veriler sabit adlı **`discord-bot-data`** Docker volume'ünde tutulur. [Güncellemeden önce yedekleyin](docs/setup-guide.md#back-up-the-memory); `docker compose pull && docker compose up -d` ile güncelleyin. Proje önekli eski volume kullanan kurulumlar için [volume taşıma adımları](docs/setup-guide.md#existing-compose-installations).

Başka yol mu tercih ediyorsunuz? [Hazır binary (terminal gerekir)](docs/setup-guide.md#download-a-release) · [Kaynaktan derleme](docs/setup-guide.md#build-from-source).

## Sınırlar ve başvuru

- Hafıza getirmenin karakter bütçesi vardır; saklanan malzeme kusursuz hatırlama demek değildir.
- Bazı kalıcı anahtarlar ve kalan gün adı/ruh hali mantığı Türkçedir. Seyahat Türk tatil takvimini kullanır ve İngilizce modda kapalıdır.
- `STEALTH`, **azaltılmış bot arayüzü** demektir; insan gibi görünmek değil. Slash etkileşimlerini/arayüz embedlerini kapatır, canlı düzenleme yerine tamamlanmış cevap gönderir. `/status` doğrulaması için kapalı bırakın.
- Promptlar güvenlik sınırı değildir. Kanal izinleri ve sağlayıcı harcama kontrollerini kullanın.

[Mimari ve ajanlar](docs/architecture.md) · [Hafıza biçimleri](docs/state-files.md) · [Yapılandırma](docs/configuration.md) · [Komutlar](docs/commands.md) · [Dokümantasyon dizini](docs/README.md)

## Katkı ve lisans

Fork alın, dal oluşturun ve pull request açın. Rust bilmeden de dokümanlar, çeviriler ve tekrarlanabilir hata raporlarıyla katkı verebilirsiniz. Bkz. [CONTRIBUTING.md](CONTRIBUTING.md).

MIT — bkz. [LICENSE](LICENSE).
