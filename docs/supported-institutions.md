# Supported banks, brokers and wallets

Which services Ferrofolio supports at all is listed in the [README](../README.md#supported-services). This page
details what each one provides. Banks without their own connector can be connected through
[open banking](#other-banks-through-open-banking).

> [!IMPORTANT]
> So far only European banks connected through open banking have been verified with Ferrofolio. The connectors
> below were taken over from Finanze and have not been verified since the fork.

In the tables, ✅ means supported, ❌ means offered by the institution but not supported yet, and - means the
institution does not offer it.

Before you connect one, read the [notes](#notes): some institutions cannot sign in on a self-hosted server.

## Products

| Institution         | Accounts        | Cards | Funds | Stocks/<br>ETFs      | Deposits | Portfolios | Loans/<br>Mortgages | Crypto | Bonds | Other                     |
| ------------------- | --------------- | ----- | ----- | -------------------- | -------- | ---------- | ------------------- | ------ | ----- | ------------------------- |
| Trade Republic      | ✅              | -     | ✅    | ✅                   | -        | -          | -                   | ✅     | ❌    | Private Markets           |
| Unicaja             | ✅              | ✅    | ❌    | ❌                   | ❌       | -          | ✅                  | -      | -     |                           |
| Wecity              | ✅              | -     | -     | -                    | -        | -          | -                   | -      | -     | Investments               |
| Mintos              | ✅ (Smart Cash) | -     | -     | Single ETFs & Stocks | -        | -          | -                   | ❌     | ❌    | Crowdlending              |
| Freedom24           | ✅              | -     | -     | ✅                   | -        | -          | -                   | ❌     | -     | Old structured placements |
| Indexa Capital      | ✅              | -     | ✅    | -                    | -        | ✅         | -                   | -      | -     |                           |
| ING                 | ✅              | ✅    | ✅    | ✅                   | ❌       | -          | ❌                  | -      | -     |                           |
| Grupo Cajamar       | ✅              | ✅    | ❌    | ❌                   | ❌       | -          | ✅                  | -      | -     |                           |
| DEGIRO              | ✅              | -     | ✅    | ✅                   | -        | -          | -                   | -      | -     |                           |
| Interactive Brokers | ✅              | -     | ❌    | ✅ Stocks            | -        | -          | -                   | -      | ❌    |                           |
| Crescenta           | -               | -     | ✅    | -                    | -        | -          | -                   | -      | -     |                           |
| Trading 212 (Beta)  | ✅              | -     | -     | ✅                   | -        | -          | -                   | ❌     | -     |                           |

| Crypto exchange | Crypto            | Other   |
| --------------- | ----------------- | ------- |
| Binance         | ✅ Spot & Futures | Futures |

## Data

Not every institution provides the same data:

- **Position:** the current state of your assets. Every institution supports it.
- **Periodic contributions:** automatic contributions to investments, such as a savings plan for stocks or ETFs.
- **Transactions:** account and investment transactions, such as interest payments, stock or fund operations,
  maturities and fees.
- **Investment history:** past and current investments built from past positions and transactions. Deposits are not
  included.

| Institution         | Position | Periodic<br>contributions | Transactions<br>(investments) | Investment<br>history |
| ------------------- | -------- | ------------------------- | ----------------------------- | --------------------- |
| Trade Republic      | ✅       | ✅                        | ✅                            | -                     |
| Unicaja             | ✅       | ✅                        | ❌                            | -                     |
| Wecity              | ✅       | -                         | ✅                            | ✅                    |
| Mintos              | ✅       | -                         | Single ETFs & Stocks          | ❌                    |
| Freedom24           | ✅       | -                         | ✅                            | ❌                    |
| Indexa Capital      | ✅       | -                         | ✅                            | -                     |
| ING                 | ✅       | ✅                        | ✅                            | -                     |
| Grupo Cajamar       | ✅       | -                         | ❌                            | -                     |
| DEGIRO              | ✅       | ❌                        | ✅                            | -                     |
| Interactive Brokers | ✅       | ❌                        | ✅ \*                         | -                     |
| Crescenta           | ✅       | -                         | ✅                            | -                     |
| Trading 212 (Beta)  | ✅       | ❌                        | ✅                            | -                     |

| Crypto exchange | Position | Periodic<br>contributions | Transactions<br>(investments) | Investment<br>history |
| --------------- | -------- | ------------------------- | ----------------------------- | --------------------- |
| Binance         | ✅ Spot  | ❌                        | ✅                            | -                     |

\* The Finanze documentation marked these transactions with a warning, without saying what is limited.

## Notes

- **Trade Republic**, **ING** and **Interactive Brokers** cannot sign in on a Ferrofolio server yet. Their sign-in
  needed a browser window that only the upstream desktop app opened, and Ferrofolio has no desktop app. A server-side
  sign-in for Trade Republic is planned, see the [roadmap](roadmap.md), section 4.7.
- **Unicaja** needs the `UNICAJA_ABCK` environment variable to sign in, because it uses Akamai bot protection.
- **Mintos** needs Selenium to solve its reCAPTCHA, see [Mintos](self-hosting.md#mintos-selenium) in the setup guide.
- **Freedom24** interest (swap) transactions of the D-Account are no longer fetched, since that account was removed.
- Some institutions need a second factor to sign in or to fetch data, so they cannot be updated in the background:
  - **Trade Republic** (mobile app or SMS)
  - **Wecity** (SMS)
  - **DEGIRO** (confirmation in the mobile app)
- **B100**, **MyInvestor**, **SEGO** and **Urbanitae** were removed. Their sign-in depended on keys served by a
  Finanze service that Ferrofolio no longer contacts. Installations that used them keep showing the positions and
  transactions stored before, but cannot fetch new data.
- **Scalable Capital** is not supported yet. A new connector is planned, see the [roadmap](roadmap.md), section 4.6.

## Other banks through open banking

Banks in the EU that Ferrofolio does not support directly can be connected through open banking (PSD2), which gives
read access to your bank accounts. Set it up under **Integrations > Providers > Banking providers**, where the app
shows the instructions.

- **Enable Banking** is the recommended provider. After you give consent, your bank sends you back to Ferrofolio; the
  address it uses is described under [Public address](self-hosting.md#public-address).
- **GoCardless** covers many EU and UK banks too, but only works if you already have an account: GoCardless no
  longer lets you create the credentials it needs.

## Crypto wallets

Add a wallet by its address and Ferrofolio tracks its current position. Transactions are not supported yet.

- **Bitcoin**, including extended public keys (xpub)
- **Ethereum**, including ERC-20 tokens
- **Litecoin**, including extended public keys (xpub)
- **Tron**, including TRC-20 tokens
- **BNB Smart Chain**, including BSC tokens. Needs an Etherscan API key, set up under **Integrations > Providers**

With a free API key from **Zerion**, Ferrofolio tracks more crypto assets and DeFi positions as well.

## Other

- **Polymarket:** current and past positions, profit and loss, and transactions, fetched from Polymarket.
- **Precious metals:** gold, silver, platinum and palladium, entered by hand and valued at the market price.
