// SPDX-License-Identifier: AGPL-3.0-only

//! Positions, from `finanze/domain/global_position.py`.
//!
//! Python builds every position with its dataclass constructor, which runs
//! `__post_init__` after assigning the fields. The Rust structs are written as
//! literals and passed through `post_init`, which derives the same fields and fails
//! where Python raises. `BaseData`, the base class of most positions, is not ported:
//! its only method, `from_dict`, is called nowhere.

use std::fmt;

use indexmap::IndexMap;
use shake::{ExtendableOutput, Shake128, Update, XofReader};
use uuid::Uuid;

use crate::commodity::{CommodityType, WeightUnit};
use crate::crypto::{AddressSource, CryptoAsset, CryptoCurrencyType, CryptoPositionType, HDWallet};
use crate::dates::Date;
use crate::datetime::DateTime;
use crate::dezimal::{ArithmeticError, Dezimal, ParseDezimalError};
use crate::earnings_expenses::FlowFrequency;
use crate::entity::Entity;
use crate::exception::{MissingFieldsError, PositionError};
use crate::external_integration::ExternalIntegrationId;
use crate::fetch_record::DataSource;
use crate::profitability::{ProfitabilityTerms, annualized_profitability};
use crate::str_enum::str_enum;

/// `ManualEntryData`: how a position the user entered is tracked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManualEntryData {
    pub tracker_key: Option<String>,
    pub track: bool,
    pub tracking_ref_outstanding: Option<Dezimal>,
    pub tracking_ref_date: Option<Date>,
}

str_enum! {
    /// `ProductType`: the kinds of positions a global position holds.
    pub enum ProductType {
        Account = "ACCOUNT",
        Card = "CARD",
        Loan = "LOAN",
        StockEtf = "STOCK_ETF",
        Fund = "FUND",
        FundPortfolio = "FUND_PORTFOLIO",
        Deposit = "DEPOSIT",
        Factoring = "FACTORING",
        RealEstateCf = "REAL_ESTATE_CF",
        Crowdlending = "CROWDLENDING",
        Crypto = "CRYPTO",
        Commodity = "COMMODITY",
        Bond = "BOND",
        Credit = "CREDIT",
        Derivative = "DERIVATIVE",
        MarketForecast = "MARKET_FORECAST",
    }
}

str_enum! {
    /// `AccountType`.
    pub enum AccountType {
        Checking = "CHECKING",
        VirtualWallet = "VIRTUAL_WALLET",
        Brokerage = "BROKERAGE",
        Savings = "SAVINGS",
        FundPortfolio = "FUND_PORTFOLIO",
    }
}

/// `Account`: the balance of a bank, brokerage or wallet account.
///
/// Its `Debug` output leaves out the IBAN, which must not reach a log.
#[derive(Clone, PartialEq)]
pub struct Account {
    pub id: Option<Uuid>,
    pub total: Dezimal,
    pub currency: String,
    pub r#type: AccountType,
    pub name: Option<String>,
    pub iban: Option<String>,
    pub interest: Option<Dezimal>,
    pub retained: Option<Dezimal>,
    pub pending_transfers: Option<Dezimal>,
    pub source: DataSource,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("total", &self.total)
            .field("currency", &self.currency)
            .field("type", &self.r#type)
            .field("name", &self.name)
            .field("iban", &self.iban.as_ref().map(|_| "<redacted>"))
            .field("interest", &self.interest)
            .field("retained", &self.retained)
            .field("pending_transfers", &self.pending_transfers)
            .field("source", &self.source)
            .finish()
    }
}

str_enum! {
    /// `CardType`.
    pub enum CardType {
        Credit = "CREDIT",
        Debit = "DEBIT",
    }
}

/// The last digits of a card, which Python types as `str | int` and keeps as given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardEnding {
    Text(String),
    Number(i64),
}

/// `Card`: the balance of a credit or debit card.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: Option<Uuid>,
    pub currency: String,
    pub r#type: CardType,
    pub used: Dezimal,
    /// `True` by default.
    pub active: bool,
    pub limit: Option<Dezimal>,
    pub name: Option<String>,
    pub ending: Option<CardEnding>,
    pub related_account: Option<Uuid>,
    pub source: DataSource,
}

impl Card {
    /// `Card(id, currency, type, used)`: an active card; the other fields take their
    /// defaults.
    pub fn new(
        id: Option<Uuid>,
        currency: impl Into<String>,
        r#type: CardType,
        used: Dezimal,
    ) -> Self {
        Card {
            id,
            currency: currency.into(),
            r#type,
            used,
            active: true,
            limit: None,
            name: None,
            ending: None,
            related_account: None,
            source: DataSource::default(),
        }
    }
}

str_enum! {
    /// `LoanType`.
    pub enum LoanType {
        Mortgage = "MORTGAGE",
        Standard = "STANDARD",
    }
}

str_enum! {
    /// `InterestType`: how the interest rate of a loan is defined; `FIXED` by default.
    #[derive(Default)]
    pub enum InterestType {
        #[default]
        Fixed = "FIXED",
        Variable = "VARIABLE",
        Mixed = "MIXED",
    }
}

str_enum! {
    /// `InstallmentFrequency`: how often a loan installment is due; `MONTHLY` by
    /// default.
    #[derive(Default)]
    pub enum InstallmentFrequency {
        Weekly = "WEEKLY",
        Biweekly = "BIWEEKLY",
        Semimonthly = "SEMIMONTHLY",
        #[default]
        Monthly = "MONTHLY",
        Bimonthly = "BIMONTHLY",
        Quarterly = "QUARTERLY",
        Semiannual = "SEMIANNUAL",
        Yearly = "YEARLY",
    }
}

impl InstallmentFrequency {
    pub const fn payments_per_year(self) -> u32 {
        match self {
            InstallmentFrequency::Weekly => 52,
            InstallmentFrequency::Biweekly => 26,
            InstallmentFrequency::Semimonthly => 24,
            InstallmentFrequency::Monthly => 12,
            InstallmentFrequency::Bimonthly => 6,
            InstallmentFrequency::Quarterly => 4,
            InstallmentFrequency::Semiannual => 2,
            InstallmentFrequency::Yearly => 1,
        }
    }

    /// `INSTALLMENT_TO_FLOW_FREQ[frequency]`: the frequency of the periodic flow that
    /// pays the installments.
    pub const fn flow_frequency(self) -> FlowFrequency {
        match self {
            InstallmentFrequency::Weekly => FlowFrequency::Weekly,
            InstallmentFrequency::Biweekly => FlowFrequency::Biweekly,
            InstallmentFrequency::Semimonthly => FlowFrequency::Semimonthly,
            InstallmentFrequency::Monthly => FlowFrequency::Monthly,
            InstallmentFrequency::Bimonthly => FlowFrequency::EveryTwoMonths,
            InstallmentFrequency::Quarterly => FlowFrequency::Quarterly,
            InstallmentFrequency::Semiannual => FlowFrequency::Semiannually,
            InstallmentFrequency::Yearly => FlowFrequency::Yearly,
        }
    }
}

/// SHAKE128 of the text, 16 bytes in lower-case hex: `hashlib.shake_128(raw.encode())
/// .hexdigest(16)`.
fn shake_128_hex(raw: &str) -> String {
    let mut hasher = Shake128::default();
    hasher.update(raw.as_bytes());
    let mut digest = [0_u8; 16];
    hasher.finalize_xof().read(&mut digest);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn loan_hash(entity_id: &str, loan_amount: Dezimal, creation_date: &str) -> String {
    let canonical_amount = loan_amount.to_fixed(2);
    shake_128_hex(&format!("{entity_id}|{canonical_amount}|{creation_date}"))
}

/// `compute_loan_hash(entity_id, loan_amount, creation_date)`: the key that tells a
/// loan apart from the entity's other loans, from its amount to the cent (rounded
/// half to even) and its creation date as given.
///
/// The amount is read like `Dezimal(str)`; `NaN`, infinities and amounts of more
/// than 38 significant digits, which Python accepts, are rejected.
pub fn compute_loan_hash(
    entity_id: &str,
    loan_amount: &str,
    creation_date: &str,
) -> Result<String, ParseDezimalError> {
    let amount = Dezimal::from_python_str(loan_amount)?;
    Ok(loan_hash(entity_id, amount, creation_date))
}

/// `compute_investment_hash(entity_id, amount, start_date, name)`: the key of a
/// manually entered investment, built like [`compute_loan_hash`].
pub fn compute_investment_hash(
    entity_id: &str,
    amount: &str,
    start_date: &str,
    name: &str,
) -> Result<String, ParseDezimalError> {
    let canonical_amount = Dezimal::from_python_str(amount)?.to_fixed(2);
    Ok(shake_128_hex(&format!(
        "{entity_id}|{canonical_amount}|{start_date}|{name}"
    )))
}

/// `Loan`: a loan or mortgage the user owes.
#[derive(Debug, Clone, PartialEq)]
pub struct Loan {
    pub id: Option<Uuid>,
    pub r#type: LoanType,
    pub currency: String,
    pub current_installment: Dezimal,
    pub interest_rate: Dezimal,
    pub loan_amount: Dezimal,
    pub creation: Date,
    pub maturity: Date,
    pub principal_outstanding: Dezimal,
    /// Always derived by [`post_init`](Self::post_init).
    pub principal_paid: Option<Dezimal>,
    pub interest_type: InterestType,
    pub installment_frequency: InstallmentFrequency,
    pub installment_interests: Option<Dezimal>,
    pub fixed_interest_rate: Option<Dezimal>,
    pub next_payment_date: Option<Date>,
    pub euribor_rate: Option<Dezimal>,
    pub fixed_years: Option<i64>,
    pub name: Option<String>,
    pub unpaid: Option<Dezimal>,
    /// Empty until [`compute_hash`](Self::compute_hash) sets it.
    pub hash: String,
    pub manual_data: Option<ManualEntryData>,
    pub source: DataSource,
}

impl Loan {
    /// `__post_init__`: the principal paid is the loan amount less the outstanding
    /// principal, whatever was given.
    pub fn post_init(mut self) -> Result<Self, ArithmeticError> {
        self.principal_paid = Some((self.loan_amount - self.principal_outstanding).value()?);
        Ok(self)
    }

    /// `compute_hash(entity_id)`: the loan's hash, computed from the entity, the
    /// amount and the creation date and kept unless one is set already.
    pub fn compute_hash(&mut self, entity_id: &str) -> &str {
        if self.hash.is_empty() {
            self.hash = loan_hash(entity_id, self.loan_amount, &self.creation.to_string());
        }
        &self.hash
    }
}

str_enum! {
    /// `AssetType`: what a fund invests in.
    pub enum AssetType {
        Equity = "EQUITY",
        FixedIncome = "FIXED_INCOME",
        MoneyMarket = "MONEY_MARKET",
        Mixed = "MIXED",
        Other = "OTHER",
    }
}

str_enum! {
    /// `FundType`.
    pub enum FundType {
        MutualFund = "MUTUAL_FUND",
        PrivateEquity = "PRIVATE_EQUITY",
        PensionFund = "PENSION_FUND",
    }
}

str_enum! {
    /// `EquityType`.
    pub enum EquityType {
        Stock = "STOCK",
        Etf = "ETF",
    }
}

/// What the `__post_init__` of stocks, funds and crypto positions derives: the
/// initial investment from the average buy price or the other way round, when the
/// amount held is not zero.
fn derive_investment(
    initial_investment: &mut Option<Dezimal>,
    average_buy_price: &mut Option<Dezimal>,
    amount: Dezimal,
) -> Result<(), ArithmeticError> {
    match (*initial_investment, *average_buy_price) {
        (None, Some(price)) if !amount.is_zero() => {
            *initial_investment = Some((price * amount).value()?);
        }
        (Some(investment), None) if !amount.is_zero() => {
            *average_buy_price = Some((investment / amount).value()?);
        }
        _ => {}
    }
    Ok(())
}

/// The `__post_init__` of stocks and funds: one of the initial investment and the
/// average buy price is required, and the other is derived from it.
fn derive_security_investment(
    initial_investment: &mut Option<Dezimal>,
    average_buy_price: &mut Option<Dezimal>,
    shares: Dezimal,
) -> Result<(), PositionError> {
    if initial_investment.is_none() && average_buy_price.is_none() {
        return Err(MissingFieldsError::new(["initial_investment", "average_buy_price"]).into());
    }
    Ok(derive_investment(
        initial_investment,
        average_buy_price,
        shares,
    )?)
}

/// `StockDetail`: shares of a stock or an ETF.
#[derive(Debug, Clone, PartialEq)]
pub struct StockDetail {
    pub id: Option<Uuid>,
    pub name: String,
    pub ticker: String,
    pub isin: String,
    pub shares: Dezimal,
    pub market_value: Dezimal,
    pub currency: String,
    pub r#type: EquityType,
    pub initial_investment: Option<Dezimal>,
    pub average_buy_price: Option<Dezimal>,
    /// Empty by default.
    pub market: String,
    pub subtype: Option<String>,
    pub info_sheet_url: Option<String>,
    pub manual_data: Option<ManualEntryData>,
    pub issuer: Option<String>,
    pub source: DataSource,
}

impl StockDetail {
    /// `__post_init__`: derives the initial investment from the average buy price
    /// or the other way round; without either it fails with `MissingFieldsError`.
    pub fn post_init(mut self) -> Result<Self, PositionError> {
        derive_security_investment(
            &mut self.initial_investment,
            &mut self.average_buy_price,
            self.shares,
        )?;
        Ok(self)
    }
}

/// `FundPortfolio`: a portfolio of funds, with its cash account.
#[derive(Debug, Clone, PartialEq)]
pub struct FundPortfolio {
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub currency: Option<String>,
    pub initial_investment: Option<Dezimal>,
    pub market_value: Option<Dezimal>,
    pub account_id: Option<Uuid>,
    pub account: Option<Account>,
    pub source: DataSource,
}

/// `FundDetail`: shares of a fund.
#[derive(Debug, Clone, PartialEq)]
pub struct FundDetail {
    pub id: Option<Uuid>,
    pub name: String,
    pub isin: String,
    pub market: Option<String>,
    pub shares: Dezimal,
    pub market_value: Dezimal,
    pub currency: String,
    pub r#type: FundType,
    pub initial_investment: Option<Dezimal>,
    pub average_buy_price: Option<Dezimal>,
    pub asset_type: Option<AssetType>,
    pub portfolio: Option<FundPortfolio>,
    pub info_sheet_url: Option<String>,
    pub manual_data: Option<ManualEntryData>,
    pub issuer: Option<String>,
    pub source: DataSource,
}

impl FundDetail {
    /// `__post_init__`, the same as [`StockDetail::post_init`].
    pub fn post_init(mut self) -> Result<Self, PositionError> {
        derive_security_investment(
            &mut self.initial_investment,
            &mut self.average_buy_price,
            self.shares,
        )?;
        Ok(self)
    }
}

/// `FactoringDetail`: an invoice financed through a factoring platform.
#[derive(Debug, Clone, PartialEq)]
pub struct FactoringDetail {
    pub id: Option<Uuid>,
    pub name: String,
    pub amount: Dezimal,
    pub currency: String,
    pub interest_rate: Dezimal,
    pub start: DateTime,
    pub maturity: Date,
    pub r#type: String,
    pub state: String,
    pub last_invest_date: Option<DateTime>,
    pub profitability: Option<Dezimal>,
    pub late_interest_rate: Option<Dezimal>,
    pub gross_interest_rate: Option<Dezimal>,
    pub gross_late_interest_rate: Option<Dezimal>,
    pub manual_data: Option<ManualEntryData>,
    pub source: DataSource,
}

impl FactoringDetail {
    /// `__post_init__`: the gross rates default to the net ones, the last
    /// investment to the start, and the profitability to the interest of the term
    /// and, once overdue, of the late days up to `today`, the local date Python
    /// reads from the clock.
    pub fn post_init(mut self, today: Date) -> Result<Self, ArithmeticError> {
        if self.gross_interest_rate.is_none() {
            self.gross_interest_rate = Some(self.interest_rate);
        }
        if self.gross_late_interest_rate.is_none() && self.late_interest_rate.is_some() {
            self.gross_late_interest_rate = self.late_interest_rate;
        }
        if self.last_invest_date.is_none() {
            self.last_invest_date = Some(self.start);
        }
        if self.profitability.is_none() {
            let terms = ProfitabilityTerms {
                interest_rate: Some(self.interest_rate),
                start_dt: Some(self.start),
                maturity: Some(self.maturity),
                late_interest_rate: self.late_interest_rate,
                ..ProfitabilityTerms::default()
            };
            self.profitability = Some(annualized_profitability(&terms, today)?);
        }
        Ok(self)
    }
}

/// `RealEstateCFDetail`: a real estate crowdfunding investment.
#[derive(Debug, Clone, PartialEq)]
pub struct RealEstateCFDetail {
    pub id: Option<Uuid>,
    pub name: String,
    pub amount: Dezimal,
    pub pending_amount: Dezimal,
    pub currency: String,
    pub interest_rate: Dezimal,
    pub start: DateTime,
    pub maturity: Date,
    pub r#type: String,
    pub state: String,
    /// Empty by default.
    pub business_type: String,
    pub last_invest_date: Option<DateTime>,
    pub profitability: Option<Dezimal>,
    pub extended_maturity: Option<Date>,
    pub extended_interest_rate: Option<Dezimal>,
    pub manual_data: Option<ManualEntryData>,
    pub source: DataSource,
}

impl RealEstateCFDetail {
    /// `__post_init__`: the last investment defaults to the start, and the
    /// profitability to the interest of the term and, once `today` has reached the
    /// maturity, of the extension.
    pub fn post_init(mut self, today: Date) -> Result<Self, ArithmeticError> {
        if self.last_invest_date.is_none() {
            self.last_invest_date = Some(self.start);
        }
        if self.profitability.is_none() {
            let terms = ProfitabilityTerms {
                interest_rate: Some(self.interest_rate),
                start_dt: Some(self.start),
                maturity: Some(self.maturity),
                extended_maturity: self.extended_maturity,
                extended_interest_rate: self.extended_interest_rate,
                ..ProfitabilityTerms::default()
            };
            self.profitability = Some(annualized_profitability(&terms, today)?);
        }
        Ok(self)
    }
}

/// `Deposit`: a term deposit.
#[derive(Debug, Clone, PartialEq)]
pub struct Deposit {
    pub id: Option<Uuid>,
    pub name: String,
    pub amount: Dezimal,
    pub currency: String,
    pub interest_rate: Dezimal,
    pub creation: DateTime,
    pub maturity: Date,
    pub expected_interests: Option<Dezimal>,
    pub source: DataSource,
}

impl Deposit {
    /// `__post_init__`: the expected interests default to the amount times the
    /// profitability of the term, rounded to cents half to even.
    pub fn post_init(mut self, today: Date) -> Result<Self, ArithmeticError> {
        if self.expected_interests.is_none() {
            let terms = ProfitabilityTerms {
                interest_rate: Some(self.interest_rate),
                start_dt: Some(self.creation),
                maturity: Some(self.maturity),
                ..ProfitabilityTerms::default()
            };
            let profitability = annualized_profitability(&terms, today)?;
            self.expected_interests = Some((self.amount * profitability).value()?.round(2)?);
        }
        Ok(self)
    }
}

/// `CryptoCurrencyPosition`: an amount of a coin or token.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoCurrencyPosition {
    pub id: Option<Uuid>,
    pub symbol: String,
    pub amount: Dezimal,
    pub r#type: CryptoCurrencyType,
    pub name: Option<String>,
    pub crypto_asset: Option<CryptoAsset>,
    pub market_value: Option<Dezimal>,
    pub currency: Option<String>,
    pub contract_address: Option<String>,
    pub initial_investment: Option<Dezimal>,
    pub average_buy_price: Option<Dezimal>,
    pub investment_currency: Option<String>,
    pub chain: Option<String>,
    pub protocol: Option<String>,
    pub position_type: CryptoPositionType,
    pub icon_url: Option<String>,
    pub source: DataSource,
}

impl CryptoCurrencyPosition {
    /// `__post_init__`: an initial investment or an average buy price needs the
    /// currency it is in, and each is derived from the other.
    pub fn post_init(mut self) -> Result<Self, PositionError> {
        let has_investment = self.initial_investment.is_some() || self.average_buy_price.is_some();
        if has_investment
            && self
                .investment_currency
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(MissingFieldsError::new(["investment_currency"]).into());
        }
        derive_investment(
            &mut self.initial_investment,
            &mut self.average_buy_price,
            self.amount,
        )?;
        Ok(self)
    }
}

/// `CryptoCurrencyWallet`: the positions found in one wallet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CryptoCurrencyWallet {
    pub id: Option<Uuid>,
    pub addresses: Vec<String>,
    pub name: Option<String>,
    pub assets: Vec<CryptoCurrencyPosition>,
    pub address_source: Option<AddressSource>,
    pub hd_wallet: Option<HDWallet>,
}

str_enum! {
    /// `CryptoInitialInvestmentType`.
    pub enum CryptoInitialInvestmentType {
        Crypto = "CRYPTO",
        Token = "TOKEN",
    }
}

/// `CryptoInitialInvestment`: what the user paid for an asset in a wallet.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoInitialInvestment {
    pub wallet_connection_id: Uuid,
    pub symbol: String,
    pub r#type: CryptoInitialInvestmentType,
    pub initial_investment: Option<Dezimal>,
    pub average_buy_price: Option<Dezimal>,
    pub investment_currency: String,
    pub currency: String,
}

str_enum! {
    /// `DerivativeContractType`.
    pub enum DerivativeContractType {
        Perpetual = "PERPETUAL",
        Futures = "FUTURES",
        KnockOut = "KNOCK_OUT",
        Factor = "FACTOR",
        Warrant = "WARRANT",
        Options = "OPTIONS",
        Cfd = "CFD",
        Other = "OTHER",
    }
}

str_enum! {
    /// `PositionDirection`.
    pub enum PositionDirection {
        Long = "LONG",
        Short = "SHORT",
    }
}

str_enum! {
    /// `MarginType`.
    pub enum MarginType {
        Cross = "CROSS",
        Isolated = "ISOLATED",
    }
}

/// `DerivativeDetail`: a derivative contract.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivativeDetail {
    pub id: Option<Uuid>,
    pub symbol: String,
    pub underlying_asset: ProductType,
    pub contract_type: DerivativeContractType,
    pub direction: PositionDirection,
    pub size: Dezimal,
    pub entry_price: Dezimal,
    pub currency: String,
    pub mark_price: Option<Dezimal>,
    pub market_value: Option<Dezimal>,
    pub unrealized_pnl: Option<Dezimal>,
    pub leverage: Option<Dezimal>,
    pub margin: Option<Dezimal>,
    pub margin_type: Option<MarginType>,
    pub liquidation_price: Option<Dezimal>,
    pub isin: Option<String>,
    pub strike_price: Option<Dezimal>,
    pub knock_out_price: Option<Dezimal>,
    pub ratio: Option<Dezimal>,
    pub issuer: Option<String>,
    pub underlying_symbol: Option<String>,
    pub underlying_isin: Option<String>,
    pub expiry: Option<Date>,
    pub name: Option<String>,
    pub initial_investment: Option<Dezimal>,
    pub source: DataSource,
}

/// `MarketForecastDetail`: a position on the outcome of a prediction market.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketForecastDetail {
    pub id: Option<Uuid>,
    pub size: Dezimal,
    pub entry_price: Dezimal,
    pub currency: String,
    pub mark_price: Option<Dezimal>,
    pub market_value: Option<Dezimal>,
    pub unrealized_pnl: Option<Dezimal>,
    pub expiry: Option<Date>,
    pub name: Option<String>,
    pub initial_investment: Option<Dezimal>,
    pub market_key: Option<String>,
    pub event_key: Option<String>,
    pub outcome_key: Option<String>,
    pub market_url: Option<String>,
    pub icon_url: Option<String>,
    pub outcome: Option<String>,
    pub source: DataSource,
}

/// `CreditDetail`: a credit line.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditDetail {
    pub id: Option<Uuid>,
    pub currency: String,
    pub credit_limit: Dezimal,
    pub drawn_amount: Dezimal,
    pub interest_rate: Dezimal,
    pub name: Option<String>,
    pub pledged_amount: Option<Dezimal>,
    pub creation: Option<Date>,
    pub source: DataSource,
}

impl CreditDetail {
    /// `available_amount`: the limit less what is drawn.
    pub fn available_amount(&self) -> Result<Dezimal, ArithmeticError> {
        (self.credit_limit - self.drawn_amount).value()
    }
}

/// `Commodity`: a [`CommodityRegister`](crate::commodity::CommodityRegister) held
/// as a position, with its fields and an id.
///
/// Python declares the id with the default factory `UUID`, which raises
/// `TypeError` when called without arguments, so a commodity always needs one.
#[derive(Debug, Clone, PartialEq)]
pub struct Commodity {
    pub name: String,
    pub r#type: CommodityType,
    pub amount: Dezimal,
    pub unit: WeightUnit,
    pub market_value: Option<Dezimal>,
    pub initial_investment: Option<Dezimal>,
    pub average_buy_price: Option<Dezimal>,
    pub currency: Option<String>,
    pub id: Uuid,
}

/// A value Python keeps untyped and no domain logic reads, held as its JSON text,
/// the form in which the database stores it and the API returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntypedJson(pub String);

/// `Crowdlending`: the summary of a crowdlending account.
#[derive(Debug, Clone, PartialEq)]
pub struct Crowdlending {
    pub id: Uuid,
    pub total: Option<Dezimal>,
    pub weighted_interest_rate: Option<Dezimal>,
    pub currency: String,
    /// A `dict` without types in Python: the amounts by loan state.
    pub distribution: Option<UntypedJson>,
    /// A `list` without types in Python, which is always empty.
    pub entries: Option<Vec<UntypedJson>>,
}

/// Declares the containers that hold the entries of one product type.
macro_rules! entries {
    ($($(#[$attribute:meta])* $name:ident($entry:ty);)+) => {$(
        $(#[$attribute])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            pub entries: Vec<$entry>,
        }
    )+};
}

entries! {
    /// `Accounts`.
    Accounts(Account);
    /// `Cards`.
    Cards(Card);
    /// `Loans`.
    Loans(Loan);
    /// `StockInvestments`.
    StockInvestments(StockDetail);
    /// `FundInvestments`.
    FundInvestments(FundDetail);
    /// `FundPortfolios`.
    FundPortfolios(FundPortfolio);
    /// `FactoringInvestments`.
    FactoringInvestments(FactoringDetail);
    /// `RealEstateCFInvestments`.
    RealEstateCFInvestments(RealEstateCFDetail);
    /// `Deposits`.
    Deposits(Deposit);
    /// `CryptoCurrencies`: the wallets of an entity.
    CryptoCurrencies(CryptoCurrencyWallet);
    /// `Commodities`.
    Commodities(Commodity);
    /// `Credits`.
    Credits(CreditDetail);
    /// `DerivativePositions`.
    DerivativePositions(DerivativeDetail);
    /// `MarketForecastPositions`.
    MarketForecastPositions(MarketForecastDetail);
}

/// `ProductPosition`: the positions of one product type, the union of the
/// containers and `Crowdlending`.
#[derive(Debug, Clone, PartialEq)]
pub enum ProductPosition {
    Accounts(Accounts),
    Cards(Cards),
    Loans(Loans),
    StockInvestments(StockInvestments),
    FundInvestments(FundInvestments),
    FundPortfolios(FundPortfolios),
    FactoringInvestments(FactoringInvestments),
    RealEstateCFInvestments(RealEstateCFInvestments),
    Deposits(Deposits),
    Crowdlending(Crowdlending),
    CryptoCurrencies(CryptoCurrencies),
    Commodities(Commodities),
    Credits(Credits),
    DerivativePositions(DerivativePositions),
    MarketForecastPositions(MarketForecastPositions),
}

/// `ProductPositions`: the positions by product type, in the order they were added.
pub type ProductPositions = IndexMap<ProductType, ProductPosition>;

/// `GlobalPosition`: everything an entity reported at one time.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalPosition {
    pub id: Uuid,
    pub entity: Entity,
    /// Set by [`post_init`](Self::post_init) when not given.
    pub date: Option<DateTime>,
    pub products: ProductPositions,
    pub source: DataSource,
    pub entity_account_id: Option<Uuid>,
}

impl GlobalPosition {
    /// `GlobalPosition(id, entity)`: a position of `now` without products.
    pub fn new(id: Uuid, entity: Entity, now: DateTime) -> Self {
        GlobalPosition {
            id,
            entity,
            date: None,
            products: ProductPositions::new(),
            source: DataSource::default(),
            entity_account_id: None,
        }
        .post_init(now)
    }

    /// `__post_init__`: the date defaults to `now`, the aware local time Python
    /// reads from the clock.
    pub fn post_init(mut self, now: DateTime) -> Self {
        if self.date.is_none() {
            self.date = Some(now);
        }
        self
    }
}

/// `HistoricalPosition`.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoricalPosition {
    pub positions: ProductPositions,
}

/// `EntitiesPosition`: the global positions by entity id.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitiesPosition {
    pub positions: IndexMap<String, Vec<GlobalPosition>>,
}

/// `PositionQueryRequest`: which positions to read; `None` does not filter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PositionQueryRequest {
    pub entities: Option<Vec<Uuid>>,
    pub excluded_entities: Option<Vec<Uuid>>,
    pub real: Option<bool>,
    pub products: Option<Vec<ProductType>>,
}

/// `CryptoEntityDetails`: the market data of a crypto asset added by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoEntityDetails {
    pub provider_asset_id: String,
    pub provider: ExternalIntegrationId,
}

/// `UpdatePositionRequest`: positions the user enters for an entity.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdatePositionRequest {
    pub products: ProductPositions,
    pub entity_id: Option<Uuid>,
    pub new_entity_name: Option<String>,
    pub new_entity_icon_url: Option<String>,
    pub net_crypto_entity_details: Option<CryptoEntityDetails>,
    /// `True` by default.
    pub create_investment_txs: bool,
}

impl UpdatePositionRequest {
    /// `UpdatePositionRequest(products)`: the other fields take their defaults.
    pub fn new(products: ProductPositions) -> Self {
        UpdatePositionRequest {
            products,
            entity_id: None,
            new_entity_name: None,
            new_entity_icon_url: None,
            net_crypto_entity_details: None,
            create_investment_txs: true,
        }
    }
}

/// `ManualPositionData`: the tracking data of a position the user entered.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualPositionData {
    pub entry_id: Uuid,
    pub global_position_id: Uuid,
    pub product_type: ProductType,
    pub data: ManualEntryData,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> Dezimal {
        Dezimal::from_python_str(value).unwrap()
    }

    fn loan(loan_amount: &str, principal_outstanding: &str) -> Loan {
        Loan {
            id: None,
            r#type: LoanType::Mortgage,
            currency: "EUR".to_owned(),
            current_installment: d("500"),
            interest_rate: d("0.03"),
            loan_amount: d(loan_amount),
            creation: "2020-01-02".parse().unwrap(),
            maturity: "2040-01-02".parse().unwrap(),
            principal_outstanding: d(principal_outstanding),
            principal_paid: Some(d("1")),
            interest_type: InterestType::default(),
            installment_frequency: InstallmentFrequency::default(),
            installment_interests: None,
            fixed_interest_rate: None,
            next_payment_date: None,
            euribor_rate: None,
            fixed_years: None,
            name: None,
            unpaid: None,
            hash: String::new(),
            manual_data: None,
            source: DataSource::default(),
        }
    }

    fn stock(
        shares: &str,
        initial_investment: Option<&str>,
        average_buy_price: Option<&str>,
    ) -> StockDetail {
        StockDetail {
            id: None,
            name: "Stock".to_owned(),
            ticker: "STK".to_owned(),
            isin: "XX0000000000".to_owned(),
            shares: d(shares),
            market_value: d("100"),
            currency: "EUR".to_owned(),
            r#type: EquityType::Stock,
            initial_investment: initial_investment.map(d),
            average_buy_price: average_buy_price.map(d),
            market: String::new(),
            subtype: None,
            info_sheet_url: None,
            manual_data: None,
            issuer: None,
            source: DataSource::default(),
        }
    }

    #[test]
    fn defaults_match_python() {
        assert_eq!(InterestType::default(), InterestType::Fixed);
        assert_eq!(
            InstallmentFrequency::default(),
            InstallmentFrequency::Monthly
        );
        let card = Card::new(None, "EUR", CardType::Debit, d("12.5"));
        assert!(card.active);
        assert_eq!(card.source, DataSource::Real);
        assert!(!ManualEntryData::default().track);
    }

    #[test]
    fn installment_frequencies() {
        assert_eq!(InstallmentFrequency::Biweekly.payments_per_year(), 26);
        assert_eq!(
            InstallmentFrequency::Bimonthly.flow_frequency(),
            FlowFrequency::EveryTwoMonths
        );
        assert_eq!(InterestType::from_value("fixed"), None);
    }

    #[test]
    fn principal_paid_is_derived() {
        let loan = loan("100.50", "50").post_init().unwrap();
        assert_eq!(loan.principal_paid, Some(d("50.50")));
    }

    #[test]
    fn loan_hash_is_kept_once_computed() {
        let mut loan = loan("100.50", "50");
        let hash = loan.compute_hash("x").to_owned();
        assert_eq!(hash, "83abe3144610487c25676151a50dccbc");
        assert_eq!(loan.compute_hash("other"), hash);
        assert_eq!(compute_loan_hash("x", "100.5", "2020-01-02").unwrap(), hash);
    }

    #[test]
    fn hashes_round_the_amount_half_to_even() {
        assert_eq!(
            compute_loan_hash("e", "100.005", "2024-01-01").unwrap(),
            "4b37c91290ba3f706d693c2b2ef0437e"
        );
        assert_eq!(
            compute_loan_hash("e", "abc", "2024-01-01"),
            Err(ParseDezimalError)
        );
    }

    #[test]
    fn securities_derive_the_missing_investment_figure() {
        let derived = stock("4", None, Some("2.5")).post_init().unwrap();
        assert_eq!(derived.initial_investment, Some(d("10")));
        let derived = stock("4", Some("10"), None).post_init().unwrap();
        assert_eq!(derived.average_buy_price, Some(d("2.5")));
        let unchanged = stock("0", Some("10"), None).post_init().unwrap();
        assert_eq!(unchanged.average_buy_price, None);
        assert_eq!(
            stock("4", None, None).post_init(),
            Err(PositionError::MissingFields(MissingFieldsError::new([
                "initial_investment",
                "average_buy_price"
            ])))
        );
    }

    #[test]
    fn account_debug_output_hides_the_iban() {
        let account = Account {
            id: None,
            total: d("10"),
            currency: "EUR".to_owned(),
            r#type: AccountType::Checking,
            name: None,
            iban: Some("ES9121000418450200051332".to_owned()),
            interest: None,
            retained: None,
            pending_transfers: None,
            source: DataSource::default(),
        };
        let printed = format!("{account:?}");
        assert!(!printed.contains("ES91"));
        assert!(printed.contains("<redacted>"));
    }

    #[test]
    fn global_positions_default_to_now() {
        let now = DateTime::naive("2026-11-18T12:00:00".parse().unwrap());
        let entity = Entity {
            id: None,
            name: "Bank".to_owned(),
            natural_id: None,
            r#type: crate::entity::EntityType::FinancialInstitution,
            origin: crate::entity::EntityOrigin::Native,
            icon_url: None,
        };
        let position = GlobalPosition::new(Uuid::nil(), entity, now);
        assert_eq!(position.date, Some(now));
        assert!(position.products.is_empty());
        let earlier = DateTime::naive("2026-01-01T00:00:00".parse().unwrap());
        let kept = GlobalPosition {
            date: Some(earlier),
            ..position
        }
        .post_init(now);
        assert_eq!(kept.date, Some(earlier));
        assert!(UpdatePositionRequest::new(ProductPositions::new()).create_investment_txs);
    }

    #[test]
    fn credit_availability() {
        let credit = CreditDetail {
            id: None,
            currency: "EUR".to_owned(),
            credit_limit: d("1000"),
            drawn_amount: d("250.5"),
            interest_rate: d("0.05"),
            name: None,
            pledged_amount: None,
            creation: None,
            source: DataSource::default(),
        };
        assert_eq!(credit.available_amount(), Ok(d("749.5")));
    }
}
