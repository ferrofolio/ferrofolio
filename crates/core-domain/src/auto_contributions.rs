// SPDX-License-Identifier: AGPL-3.0-only

//! Periodic contributions, from `finanze/domain/auto_contributions.py`.

use indexmap::IndexMap;
use uuid::Uuid;

use crate::Dezimal;
use crate::dates::Date;
use crate::entity::Entity;
use crate::fetch_record::DataSource;
use crate::str_enum::str_enum;

str_enum! {
    /// `ContributionFrequency`: how often a contribution recurs.
    pub enum ContributionFrequency {
        Weekly = "WEEKLY",
        Biweekly = "BIWEEKLY",
        Monthly = "MONTHLY",
        Bimonthly = "BIMONTHLY",
        EveryFourMonths = "EVERY_FOUR_MONTHS",
        Quarterly = "QUARTERLY",
        Semiannual = "SEMIANNUAL",
        Yearly = "YEARLY",
    }
}

str_enum! {
    /// `ContributionTargetType`: what the contribution pays for.
    pub enum ContributionTargetType {
        StockEtf = "STOCK_ETF",
        Fund = "FUND",
        FundPortfolio = "FUND_PORTFOLIO",
        Crypto = "CRYPTO",
    }
}

str_enum! {
    /// `ContributionTargetSubtype`: the kind of stock or fund target.
    pub enum ContributionTargetSubtype {
        Stock = "STOCK",
        Etf = "ETF",
        MutualFund = "MUTUAL_FUND",
        PrivateEquity = "PRIVATE_EQUITY",
        PensionFund = "PENSION_FUND",
    }
}

/// `PeriodicContribution`: a recurring payment into an investment.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodicContribution {
    pub id: Uuid,
    pub alias: Option<String>,
    pub target: String,
    pub target_name: String,
    pub target_type: ContributionTargetType,
    pub target_subtype: Option<ContributionTargetSubtype>,
    pub amount: Dezimal,
    pub currency: String,
    pub since: Date,
    pub until: Option<Date>,
    pub frequency: ContributionFrequency,
    pub active: bool,
    pub source: DataSource,
    pub next_date: Option<Date>,
    pub entity: Entity,
    pub entity_account_id: Option<Uuid>,
}

/// `AutoContributions`: the periodic contributions of one entity.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AutoContributions {
    pub periodic: Vec<PeriodicContribution>,
}

/// `EntityContributions`: the contributions by entity id, in the order the rows
/// were read, as Python's `dict` keeps the order of insertion.
pub type EntityContributions = IndexMap<String, AutoContributions>;

/// `ContributionQueryRequest`: the filters of the route. An empty filter is `None`,
/// as the route passes it to the use case.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContributionQueryRequest {
    pub entities: Option<Vec<String>>,
    pub excluded_entities: Option<Vec<Uuid>>,
    pub real: Option<bool>,
}
