# Features

A tour of what Ferrofolio does. Which banks, brokers and wallets it connects to is listed in
[Supported banks, brokers and wallets](supported-institutions.md).

- [Your whole net worth](#your-whole-net-worth)
- [Activity and budgets](#activity-and-budgets)
- [Labels, rules and AI](#labels-rules-and-ai)
- [Real estate](#real-estate)
- [Savings and retirement calculator](#savings-and-retirement-calculator)
- [Crypto and precious metals](#crypto-and-precious-metals)
- [Import and export](#import-and-export)
- [AI providers](#ai-providers)
- [MCP server](#mcp-server)
- [On your phone](#on-your-phone)

## Your whole net worth

Ferrofolio fetches your positions from banks, brokers, crowdfunding platforms, crypto exchanges and wallets, or you
enter and import them yourself. It supports accounts, cards, stocks, ETFs, funds, deposits, loans and mortgages,
bonds, crypto, real estate, crowdfunding and commodities, in any currency: exchange rates are fetched automatically.

The dashboard shows your total value, how it splits by product and by source, how it changed over time, upcoming
payments and ongoing investments. Periodic contributions, such as a monthly ETF savings plan, are used to forecast
your future positions.

<p align="center"><img src="../resources/integrations.png" alt="Connected banks, brokers and open banking accounts" width="80%" /></p>

## Activity and budgets

The activity view shows income, expenses and your savings rate, a daily cashflow heatmap and spending by label.

<p align="center"><img src="../resources/activity.png" alt="Activity analysis with income, expenses, savings rate, a daily cashflow heatmap and spending by label" width="80%" /></p>

Cashflow budgets set monthly targets for needs, wants and investment contributions and compare them with your actual
cashflow. You see the amount still available or exceeded, follow the remaining income over time and drill into
spending by label. Recurring earnings and expenses can be tracked as well.

<p align="center"><img src="../resources/budget.png" alt="Earnings and expenses tracking with contribution forecasting" width="80%" /></p>

## Labels, rules and AI

Labels sort your transactions into categories. Rules apply them for you: a rule can match the transaction text,
type, amount, currency, dates, entities or IBANs. You can preview what a rule matches before applying it, also to
existing transactions.

An [AI provider](#ai-providers) can suggest labels as well. Models that support it also return how confident they
are in each suggestion.

## Real estate

Track your home and rental properties with their value, rent, costs and mortgage. Ferrofolio works out the monthly
and annual cashflow and metrics such as return on investment, cash-on-cash return and return on equity.

<p align="center"><img src="../resources/realestate.png" alt="Real estate investment tracking with KPIs and metrics" width="80%" /></p>

## Savings and retirement calculator

Compare savings plans side by side. Each scenario takes a starting amount, a periodic contribution and an expected
return, and works out the final balance, the contribution needed for a target amount or the years until you reach
it. An optional retirement phase adds withdrawals and shows how long the money lasts.

<p align="center"><img src="../resources/savingscalculator.png" alt="Savings and retirement calculator with multiple scenarios" width="80%" /></p>

## Crypto and precious metals

Add a crypto wallet by its address, or connect an exchange, and Ferrofolio tracks its balance. Some brokers, such as
Trade Republic, hold crypto too. The supported networks are listed in
[Supported banks, brokers and wallets](supported-institutions.md#crypto-wallets).

<p align="center"><img src="../resources/crypto.png" alt="Crypto portfolio across wallets and exchanges" width="80%" /></p>

Gold, silver, platinum and palladium are entered by hand and valued at the current market price.

## Import and export

- **Files.** Export to CSV, TSV and Excel. Import from CSV, TSV, Excel (XLS and XLSX), HTML tables and text-based
  PDFs. Columns are detected automatically, or mapped by hand or with a saved template, with a preview before saving.
- **AI import.** An [AI provider](#ai-providers) extracts positions and transactions without a template from the
  formats above, from plain text and from images (PNG, JPEG, WebP, GIF, plus HEIC on mobile), including scanned PDFs.
  You can add instructions and fallback providers. Images and scanned PDFs need a model that can read images.
  Results are previewed before saving.
- **Google Sheets.** Export and import by connecting a Google account under **Integrations > Providers**. Imported
  spreadsheet data is its own source, so each import replaces the previous Google Sheets data without touching data
  from your banks or entered by hand. Google only accepts certain server addresses, see
  [Public address](self-hosting.md#public-address).

Templates define the structure of the tables, the formatting of the data, filters and more. They are optional for
file imports and define Google Sheets imports.

<p align="center"><img src="../resources/exporttemplates.png" alt="Export and import templates" width="80%" /></p>

## AI providers

AI labels transactions and imports files. It is optional; everything else works without it. Connect a provider under
**Integrations > Providers > AI providers**:

- OpenAI with an API key
- ChatGPT with a Plus or Pro plan
- OpenRouter
- Anthropic Claude through Claude Code
- A custom OpenAI-compatible endpoint, such as Ollama, LM Studio, vLLM or a single-model llama.cpp server, with its
  base URL and an optional API key
- Local models in GGUF format, run by a llama.cpp server in router mode. Ferrofolio downloads recommended models or
  models from Hugging Face, or uses GGUF files you add yourself. Models with a vision projector (`mmproj`) can read
  images and scanned PDFs. See [Local AI models](self-hosting.md#local-ai-models) for the setup.

With a local model or a self-hosted endpoint your data stays on your own hardware. Endpoints have to be reachable from
the Ferrofolio server.

## MCP server

Ferrofolio includes a [Model Context Protocol](https://modelcontextprotocol.io) server that lets AI assistants which
support it read your Ferrofolio data and use its tools. Open **Settings > MCP** to enable it, choose an access mode and
copy the connection snippet for your client. Access is read-only by default. Write access allows the assistant to
create, edit and delete data, connect integrations and fetch data from your banks.

## On your phone

The web app adapts to small screens, with a customisable navigation bar at the bottom. Open it in your phone's
browser; see [Use it from other devices](self-hosting.md#use-it-from-other-devices).

<p align="center">
  <img src="../resources/m-stocks.png" alt="Stocks and ETFs on a phone" width="24%" />
  <img src="../resources/m-funds.png" alt="Funds on a phone" width="24%" />
  <img src="../resources/m-txs.png" alt="Transactions on a phone" width="24%" />
</p>
