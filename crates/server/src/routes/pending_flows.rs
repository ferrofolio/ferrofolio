// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/flows/pending`, ported from
//! `finanze/infrastructure/controller/routes/get_pending_flows.py`.
//!
//! Python reads the flows from the database on every request, as Rust does, so a
//! flow Python creates, changes, settles or deletes is read by the next request to
//! either. A flow stored in a form Python's writer does not produce goes to Python
//! (`ReadError::Unreadable`), and so do a `page` or `limit` outside the range of
//! `i64` and statistics whose sum a float cannot hold.

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use core_app::ports::ReadError;
use core_app::use_cases::query_pending_flows::QueryPendingFlows;
use core_domain::Dezimal;
use core_domain::earnings_expenses::{
    CategoryStat, FlowSortField, FlowStatus, FlowType, FlowTypeStat, PendingFlow, PendingFlowPage,
    PendingFlowQuery, SortOrder,
};
use infra_db::Database;
use infra_db::pending_flows::PendingFlowRepository;
use serde_json::{Map, Value, json};

use crate::app::AppState;
use crate::error::ApiError;
use crate::pycompat::{self, PyInt};
use crate::routes::database::{self, Reply};
use crate::routes::periodic_flows;

pub const PATH: &str = "/api/v1/flows/pending";

/// `GET /api/v1/flows/pending`: the pending flows, filtered, sorted and paged by
/// the query.
pub async fn get_pending_flows(State(state): State<AppState>, request: Request) -> Response {
    database::serve(&state, request, prepare).await
}

/// The query as the Python route reads it, or why Rust leaves it to Python.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedQuery {
    Query(PendingFlowQuery),
    /// A `page` or `limit` outside the range of `i64`.
    PassOn(&'static str),
}

fn prepare(
    _state: &AppState,
    parts: &Parts,
) -> Result<impl FnOnce(&Database) -> Reply + Send + use<>, ApiError> {
    let parsed = parse_query(parts.uri.query().unwrap_or_default())?;
    Ok(move |database: &Database| {
        let query = match parsed {
            ParsedQuery::Query(query) => query,
            ParsedQuery::PassOn(reason) => return Reply::PassOn(reason.to_owned()),
        };
        let repository = PendingFlowRepository::new(database);
        match QueryPendingFlows::new(&repository).execute(&query) {
            Ok(page) => match page_json(&page) {
                Some(body) => Reply::Answer(respond(&body)),
                None => Reply::PassOn("a sum a float cannot hold".to_owned()),
            },
            Err(ReadError::Unreadable(reason)) => Reply::PassOn(reason),
            Err(ReadError::Failed(error)) => {
                tracing::error!(%error, "reading the pending flows failed");
                Reply::Answer(ApiError::Unexpected(error.0).into_response())
            }
        }
    })
}

/// `Enum(value)` for each value, failing with Python's message at the first one
/// that is not a member.
fn member<T>(value: &str, parse: fn(&str) -> Option<T>, name: &str) -> Result<T, ApiError> {
    parse(value).ok_or_else(|| {
        ApiError::InvalidRequest(format!(
            "{} is not a valid {name}",
            pycompat::repr_str(value)
        ))
    })
}

/// `args.get(name, type=int)`: `None` for a missing value or one `int` rejects.
fn int_arg(query: &str, name: &str) -> Result<Option<i64>, &'static str> {
    match pycompat::query_values(query, name)
        .first()
        .map(|value| pycompat::py_int(value))
    {
        None | Some(PyInt::Invalid) => Ok(None),
        Some(PyInt::Value(value)) => Ok(Some(value)),
        Some(PyInt::OutOfRange) => Err("a page or limit outside the range of i64"),
    }
}

/// `args.get(name, "false").lower() == "true"`.
fn flag_arg(query: &str, name: &str) -> bool {
    pycompat::query_values(query, name)
        .first()
        .is_some_and(|value| value.to_lowercase() == "true")
}

/// The first value of `name` unless it is empty, as `args.get(name)` and a truth
/// test read it.
fn first_non_empty(query: &str, name: &str) -> Option<String> {
    pycompat::query_values(query, name)
        .into_iter()
        .next()
        .filter(|value| !value.is_empty())
}

/// The route's reading of `request.args`, in its order: the statuses, the flow
/// type, the categories, the sort field and the order, each failing with `400`;
/// then `page` and `limit`, which fall back to `None`, and the two flags.
pub fn parse_query(query: &str) -> Result<ParsedQuery, ApiError> {
    let statuses = pycompat::query_values(query, "status")
        .iter()
        .map(|value| member(value, FlowStatus::parse, "FlowStatus"))
        .collect::<Result<Vec<_>, _>>()?;
    let flow_type = first_non_empty(query, "flow_type")
        .map(|value| member(&value, FlowType::parse, "FlowType"))
        .transpose()?;
    let categories = pycompat::query_values(query, "category");
    let sort_by = first_non_empty(query, "sort_by")
        .map(|value| member(&value, FlowSortField::parse, "FlowSortField"))
        .transpose()?
        .unwrap_or(FlowSortField::Amount);
    let order = first_non_empty(query, "order")
        .map(|value| member(&value, SortOrder::parse, "SortOrder"))
        .transpose()?
        .unwrap_or(SortOrder::Desc);
    let (page, limit) = match (int_arg(query, "page"), int_arg(query, "limit")) {
        (Ok(page), Ok(limit)) => (page, limit),
        (Err(reason), _) | (_, Err(reason)) => return Ok(ParsedQuery::PassOn(reason)),
    };
    Ok(ParsedQuery::Query(PendingFlowQuery {
        statuses: (!statuses.is_empty()).then_some(statuses),
        flow_type,
        categories: (!categories.is_empty()).then_some(categories),
        sort_by,
        order,
        page,
        limit,
        include_stats: flag_arg(query, "stats"),
        include_categories: flag_arg(query, "categories"),
    }))
}

/// An amount as `jsonify` writes a `Dezimal`: a float, `None` where the float is
/// not finite and Python would write `Infinity`.
fn amount(amount: Dezimal) -> Option<Value> {
    let float = amount.to_f64_for_api();
    float.is_finite().then(|| json!(float))
}

/// A dictionary of amounts by currency; `jsonify` sorts its keys.
fn amounts(amounts: &[(String, Dezimal)]) -> Option<Value> {
    let mut map = Map::new();
    for (currency, value) in amounts {
        map.insert(currency.clone(), amount(*value)?);
    }
    Some(Value::Object(map))
}

/// One flow as `_serialize_flow` builds it.
pub fn flow_json(flow: &PendingFlow) -> Option<Value> {
    Some(json!({
        "id": flow.id,
        "name": flow.name,
        "amount": amount(flow.amount)?,
        "currency": flow.currency,
        "flow_type": flow.flow_type.value(),
        "category": flow.category,
        "status": flow.status.value(),
        "status_changed_at": flow.status_changed_at.as_ref().map(|time| time.isoformat()),
        "date": periodic_flows::iso_date(flow.date),
        "icon": flow.icon,
    }))
}

fn category_json(stat: &CategoryStat) -> Option<Value> {
    Some(json!({
        "category": stat.category,
        "name": stat.name,
        "amounts": amounts(&stat.amounts)?,
    }))
}

fn type_json(stat: &FlowTypeStat) -> Option<Value> {
    Some(json!({
        "totals": amounts(&stat.totals)?,
        "count": stat.count,
        "by_category": stat
            .by_category
            .iter()
            .map(category_json)
            .collect::<Option<Vec<_>>>()?,
    }))
}

/// `_serialize_page`, `None` when an amount is not finite as a float.
pub fn page_json(page: &PendingFlowPage) -> Option<Value> {
    let stats = match &page.stats {
        None => Value::Null,
        Some(stats) => json!({
            "earning": type_json(&stats.earning)?,
            "expense": type_json(&stats.expense)?,
        }),
    };
    Some(json!({
        "entries": page.entries.iter().map(flow_json).collect::<Option<Vec<_>>>()?,
        "total": page.total,
        "page": page.page,
        "limit": page.limit,
        "has_more": page.has_more,
        "categories": page.categories,
        "stats": stats,
    }))
}

/// `jsonify(...)`: sorted keys and a newline.
fn respond(body: &Value) -> Response {
    ([(CONTENT_TYPE, "application/json")], format!("{body}\n")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str) -> PendingFlowQuery {
        match parse_query(text) {
            Ok(ParsedQuery::Query(query)) => query,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn error(text: &str) -> String {
        match parse_query(text) {
            Err(ApiError::InvalidRequest(message)) => message,
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn reads_the_query_like_the_python_route() {
        assert_eq!(query(""), PendingFlowQuery::default());
        let read = query(
            "status=ACTIVE&status=COMPLETED&flow_type=EARNING&flow_type=x&category=a&category=\
             &sort_by=date&order=asc&page=%202&limit=x&stats=TRUE&categories=1",
        );
        assert_eq!(
            read.statuses,
            Some(vec![FlowStatus::Active, FlowStatus::Completed])
        );
        assert_eq!(read.flow_type, Some(FlowType::Earning));
        assert_eq!(read.categories, Some(vec!["a".to_owned(), String::new()]));
        assert_eq!(read.sort_by, FlowSortField::Date);
        assert_eq!(read.order, SortOrder::Asc);
        assert_eq!((read.page, read.limit), (Some(2), None));
        assert!(read.include_stats);
        assert!(!read.include_categories);
        let empty = query("flow_type=&sort_by=&order=&page=&stats");
        assert_eq!(empty, PendingFlowQuery::default());
    }

    #[test]
    fn fails_with_the_first_invalid_member() {
        assert_eq!(error("order=up&status=x"), "'x' is not a valid FlowStatus");
        assert_eq!(error("status="), "'' is not a valid FlowStatus");
        assert_eq!(
            error("order=up&flow_type=it's"),
            "\"it's\" is not a valid FlowType"
        );
        assert_eq!(
            error("order=up&sort_by=name"),
            "'name' is not a valid FlowSortField"
        );
        assert_eq!(error("order=DESC"), "'DESC' is not a valid SortOrder");
    }

    #[test]
    fn leaves_huge_pages_to_python() {
        assert!(matches!(
            parse_query("limit=99999999999999999999"),
            Ok(ParsedQuery::PassOn(_))
        ));
        assert!(matches!(
            parse_query("page=-9223372036854775809&limit=1"),
            Ok(ParsedQuery::PassOn(_))
        ));
    }
}
