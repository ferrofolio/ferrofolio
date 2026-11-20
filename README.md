<p align="center"><img src="frontend/app/public/ferrofolio-fg.svg" alt="Ferrofolio logo" width="140px"></p>
<h1 align="center">Ferrofolio</h1>

<p align="center"><b>All your money in one place, on your own terms.</b></p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-blue?logo=gnu&logoColor=white" alt="License"></a>
  <img src="https://img.shields.io/badge/status-not%20production%20ready-orange" alt="Project status">
  <a href="https://github.com/ferrofolio/ferrofolio/commits/develop"><img src="https://img.shields.io/github/last-commit/ferrofolio/ferrofolio/develop" alt="Last commit on develop"></a>
  <a href="https://github.com/ferrofolio/ferrofolio/issues"><img src="https://img.shields.io/github/issues/ferrofolio/ferrofolio" alt="Open issues"></a>
  <a href="https://github.com/ferrofolio/ferrofolio/pulls"><img src="https://img.shields.io/github/issues-pr/ferrofolio/ferrofolio" alt="Open pull requests"></a>
  <br>
  <img src="https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fferrofolio%2Fferrofolio%2Fdevelop%2Frust-toolchain.toml&query=%24.toolchain.channel&label=Rust&color=CE422B&logo=rust&logoColor=white" alt="Rust version">
  <img src="https://img.shields.io/github/package-json/dependency-version/ferrofolio/ferrofolio/dev/react/develop?filename=frontend%2Fapp%2Fpackage.json&label=React&color=61DAFB&logo=react&logoColor=black" alt="React version">
</p>

<p align="center">
See your bank accounts, investments, crypto and property side by side, find out where your money goes and plan
ahead. Ferrofolio runs on your own computer or home server and keeps your data there, encrypted.
</p>

<p align="center"><sub>
An independent, community-driven fork of <a href="https://github.com/finanze/finanze">Finanze</a>,
not affiliated with or endorsed by the Finanze project.
</sub></p>

> [!WARNING]
> **Ferrofolio is not production ready.** The project is currently under heavy rewrite and is not reliable yet.
> Expect bugs, missing features and updates that break things, and do not rely on it as the only record of your
> finances.

<p align="center">
  <img src="resources/dashboard.png" alt="Ferrofolio dashboard showing total net worth, assets by type and net worth over time" width="74%" />
  <img src="resources/m-dashboard.png" alt="Ferrofolio dashboard on a phone" width="22%" />
</p>

## What you can do

- 🏦 **See everything at once.** Bank accounts, cards, stocks, ETFs, funds, deposits, loans, crypto, real estate and
  precious metals in one overview.
- 🔄 **Stop typing numbers.** Connect your banks and brokers, and Ferrofolio fetches balances and transactions for you.
- 💸 **Know where your money goes.** Income, spending and savings rate, sorted into your own categories, with help
  from AI if you want it.
- 🎯 **Stay on budget.** Set monthly targets for needs, wants and investments and see what is left.
- 📈 **Plan ahead.** Forecast your net worth and try out savings and retirement scenarios.
- 📥 **Bring your own data.** Import statements from Excel, CSV or PDF, or let AI read them for you. Export to Excel
  or Google Sheets.

More in the [feature guide](docs/features.md).

## Your data stays yours

- 🏠 **Private.** Ferrofolio runs on your own hardware. There is no Ferrofolio account and no cloud service behind it.
- 🔐 **Encrypted.** Your financial data is stored in an encrypted database that only your key unlocks.
- 👀 **Read-only.** Ferrofolio only reads from your banks and brokers. It cannot move money or place orders.
- 💚 **Free and open source.** No subscription, no ads, and anyone can check the code.

## Supported services

✅ implemented · ⚠️ planned · ❌ not available

So far only European banks connected through open banking have been verified with Ferrofolio. Everything else
marked ✅ was taken over from Finanze and has not been verified since the fork.

### Banks and brokers

| Service | Type | Status | Notes |
|---|---|---|---|
| **Open banking** via Enable Banking | European banks | ✅ Verified | For banks without their own connector |
| **Open banking** via GoCardless | EU and UK banks | ✅ | Only with an existing GoCardless account |
| B100 | Bank | ❌ | Removed, its sign-in depended on a Finanze service |
| Crescenta | Investment platform | ✅ | |
| DEGIRO | Broker | ✅ | Sign-in needs a confirmation in the DEGIRO app |
| Freedom24 | Broker | ✅ | |
| Grupo Cajamar | Bank | ✅ | |
| Indexa Capital | Robo-advisor | ✅ | |
| ING | Bank | ❌ | Its sign-in only worked in the removed desktop app |
| Interactive Brokers | Broker | ❌ | Its sign-in only worked in the removed desktop app |
| Mintos | P2P lending | ✅ | Needs the optional Selenium service |
| MyInvestor | Bank | ❌ | Removed, its sign-in depended on a Finanze service |
| Scalable Capital | Broker | ⚠️ | New connector |
| SEGO | Crowdfunding | ❌ | Removed, its sign-in depended on a Finanze service |
| Trade Republic | Broker | ⚠️ | Cannot sign in on a server yet, a new sign-in is planned |
| Trading 212 | Broker | ✅ | Beta |
| Unicaja | Bank | ✅ | Needs the `UNICAJA_ABCK` setting |
| Urbanitae | Real estate crowdfunding | ❌ | Removed, its sign-in depended on a Finanze service |
| Wecity | Real estate crowdfunding | ✅ | Sign-in needs a code by SMS |

### Crypto and more

| Service | Type | Status | Notes |
|---|---|---|---|
| Binance | Crypto exchange | ✅ | Spot and futures |
| Bitcoin | Crypto wallet | ✅ | Address or xpub |
| BNB Smart Chain | Crypto wallet | ✅ | Including tokens, needs an Etherscan API key |
| Ethereum | Crypto wallet | ✅ | Including ERC-20 tokens |
| Gold, silver, platinum, palladium | Precious metals | ✅ | Entered by hand, valued at the market price |
| Litecoin | Crypto wallet | ✅ | Address or xpub |
| Polymarket | Prediction market | ✅ | |
| Tron | Crypto wallet | ✅ | Including TRC-20 tokens |
| Zerion | Crypto portfolio | ✅ | More crypto assets and DeFi, with a free API key |
| Transactions of crypto wallets | Crypto wallet | ⚠️ | Wallets show their balance only for now |

What each service provides, such as accounts, funds or transactions, is listed in the
[details](docs/supported-institutions.md).

## Get started

Ferrofolio is a young project and there is no ready-made download yet. You run it with
[Docker](https://docs.docker.com/get-started/get-docker/) on a computer that you leave switched on, such as a home
server or a NAS.

1. Download Ferrofolio and start it. The first start takes a few minutes.

   ```sh
   git clone https://github.com/ferrofolio/ferrofolio.git
   cd ferrofolio
   docker compose up -d --build
   ```

2. Open <http://localhost:8080> in your browser.
3. Create your account. Choose your encryption key carefully and keep it safe: without it, nobody can open your
   data, not even you.

To use Ferrofolio from your phone or another computer, to update it, or to back up your data, see the
[setup guide](docs/self-hosting.md).

## Learn more

- [Features](docs/features.md): budgets, labels, AI, import and export, and more
- [Supported banks, brokers and wallets](docs/supported-institutions.md)
- [Setup guide](docs/self-hosting.md): other devices, updates, backups and settings
- [Contributing](.github/CONTRIBUTING.md) and the [developer guide](docs/development.md)
- [Roadmap](docs/roadmap.md): where the project is heading

## License

Ferrofolio is free software licensed under the [GNU Affero General Public License v3.0](./LICENSE) (`AGPL-3.0-only`).

Contributions are accepted under the same license. There is no contributor license agreement: nobody, including
the maintainers, can relicense the project under proprietary terms. See [CONTRIBUTING](.github/CONTRIBUTING.md).

Ferrofolio is derived from [Finanze](https://github.com/finanze/finanze). See [NOTICE](./NOTICE).

## Attributions & Credits

- Powered by [CoinGecko](https://www.coingecko.com/).
- Powered by [CoinMarketCap](https://coinmarketcap.com/).
- Powered by [CryptoCompare](https://www.cryptocompare.com/).
- Llama.cpp icon from [ggml-org/llama.brand](https://github.com/ggml-org/llama.brand), used under
  its [brand usage policy](https://github.com/ggml-org/llama.brand/blob/master/BRAND-USAGE.md) (CC BY-NC 4.0).
- Trade Republic client is based on project [pytr-org/pytr](https://github.com/pytr-org/pytr), although it has been
  heavily modified, this library has been vital for the entity integration.
- Selenium reCAPTCHA resolution is based
  on [sarperavci/GoogleRecaptchaBypass](https://github.com/sarperavci/GoogleRecaptchaBypass/tree/selenium)
  project, using a slightly modified version of Selenium version.
