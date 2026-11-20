// SPDX-License-Identifier: AGPL-3.0-only

//! Transactions, the part of `finanze/domain/transactions.py` the ported routes
//! read.

/// `TxType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxType {
    Buy,
    Sell,
    Dividend,
    RightIssue,
    RightSell,
    Subscription,
    Split,
    SwapFrom,
    SwapTo,
    TransferIn,
    TransferOut,
    SwitchFrom,
    SwitchTo,
    Investment,
    Repayment,
    Interest,
    Fee,
    Inflow,
    Outflow,
}

impl TxType {
    /// Every member, in the order Python declares them.
    pub const ALL: [TxType; 19] = [
        TxType::Buy,
        TxType::Sell,
        TxType::Dividend,
        TxType::RightIssue,
        TxType::RightSell,
        TxType::Subscription,
        TxType::Split,
        TxType::SwapFrom,
        TxType::SwapTo,
        TxType::TransferIn,
        TxType::TransferOut,
        TxType::SwitchFrom,
        TxType::SwitchTo,
        TxType::Investment,
        TxType::Repayment,
        TxType::Interest,
        TxType::Fee,
        TxType::Inflow,
        TxType::Outflow,
    ];

    pub fn value(self) -> &'static str {
        match self {
            TxType::Buy => "BUY",
            TxType::Sell => "SELL",
            TxType::Dividend => "DIVIDEND",
            TxType::RightIssue => "RIGHT_ISSUE",
            TxType::RightSell => "RIGHT_SELL",
            TxType::Subscription => "SUBSCRIPTION",
            TxType::Split => "SPLIT",
            TxType::SwapFrom => "SWAP_FROM",
            TxType::SwapTo => "SWAP_TO",
            TxType::TransferIn => "TRANSFER_IN",
            TxType::TransferOut => "TRANSFER_OUT",
            TxType::SwitchFrom => "SWITCH_FROM",
            TxType::SwitchTo => "SWITCH_TO",
            TxType::Investment => "INVESTMENT",
            TxType::Repayment => "REPAYMENT",
            TxType::Interest => "INTEREST",
            TxType::Fee => "FEE",
            TxType::Inflow => "INFLOW",
            TxType::Outflow => "OUTFLOW",
        }
    }

    /// `TxType(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|member| member.value() == value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exactly_the_values_of_its_members() {
        for member in TxType::ALL {
            assert_eq!(TxType::parse(member.value()), Some(member));
        }
        assert_eq!(TxType::parse("inflow"), None);
        assert_eq!(TxType::parse(" INFLOW"), None);
        assert_eq!(TxType::parse(""), None);
    }
}
