// SPDX-License-Identifier: AGPL-3.0-only

//! External integrations from `finanze/domain/external_integration.py`, ported as
//! they are needed.

use crate::str_enum::str_enum;

str_enum! {
    /// `ExternalIntegrationId`: a third-party service the user can connect.
    pub enum ExternalIntegrationId {
        GoogleSheets = "GOOGLE_SHEETS",
        Etherscan = "ETHERSCAN",
        Ethplorer = "ETHPLORER",
        Zerion = "ZERION",
        Gocardless = "GOCARDLESS",
        EnableBanking = "ENABLE_BANKING",
        Coingecko = "COINGECKO",
        Coinmarketcap = "COINMARKETCAP",
        Cryptocompare = "CRYPTOCOMPARE",
        Openrouter = "OPENROUTER",
        Openai = "OPENAI",
        OpenaiCompatible = "OPENAI_COMPATIBLE",
        LocalModels = "LOCAL_MODELS",
        GithubCopilot = "GITHUB_COPILOT",
        ClaudeAgent = "CLAUDE_AGENT",
        Chatgpt = "CHATGPT",
    }
}
