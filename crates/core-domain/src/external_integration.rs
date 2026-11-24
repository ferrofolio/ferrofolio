// SPDX-License-Identifier: AGPL-3.0-only

//! External integrations from `finanze/domain/external_integration.py`, ported as
//! they are needed.

use std::fmt;

use indexmap::IndexMap;

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
        ClaudeAgent = "CLAUDE_AGENT",
        Chatgpt = "CHATGPT",
    }
}

/// `ExternalIntegrationPayload`: what connects an integration, such as an API key.
///
/// Python's type is a plain `dict[str, str]`. The values are credentials, so the
/// `Debug` output of this map shows its keys only, and a payload cannot reach a log
/// through the `Debug` output of a type that holds it.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ExternalIntegrationPayload(pub IndexMap<String, String>);

impl fmt::Debug for ExternalIntegrationPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key, "<redacted>")))
            .finish()
    }
}

/// `EnabledExternalIntegrations`: the payload of every connected integration.
pub type EnabledExternalIntegrations = IndexMap<ExternalIntegrationId, ExternalIntegrationPayload>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_credentials() {
        let payload = ExternalIntegrationPayload(IndexMap::from([(
            "api_key".to_owned(),
            "secret-value".to_owned(),
        )]));
        let integrations =
            EnabledExternalIntegrations::from([(ExternalIntegrationId::Etherscan, payload)]);
        let printed = format!("{integrations:?}");
        assert!(printed.contains("api_key"));
        assert!(!printed.contains("secret-value"));
    }
}
