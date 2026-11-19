// SPDX-License-Identifier: AGPL-3.0-only

//! Positions, from `finanze/domain/global_position.py`.
//!
//! Python builds every position with its dataclass constructor, which runs
//! `__post_init__` after assigning the fields. The Rust structs are written as
//! literals and passed through `post_init`, which derives the same fields and fails
//! where Python raises. `BaseData`, the base class of most positions, is not ported:
//! its only method, `from_dict`, is called nowhere.

use std::fmt;

use shake::{ExtendableOutput, Shake128, Update, XofReader};
use uuid::Uuid;

use crate::dates::Date;
use crate::dezimal::{ArithmeticError, Dezimal, ParseDezimalError};
use crate::earnings_expenses::FlowFrequency;
use crate::exception::{MissingFieldsError, PositionError};
use crate::fetch_record::DataSource;
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
}
