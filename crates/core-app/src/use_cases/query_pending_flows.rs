// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/query_pending_flows.py`.

use core_domain::Dezimal;
use core_domain::earnings_expenses::{
    CategoryStat, FlowSortField, FlowStatus, FlowType, FlowTypeStat, PendingFlow, PendingFlowPage,
    PendingFlowQuery, PendingFlowStats, SortOrder,
};

use crate::ports::{PendingFlowPort, ReadError};

/// `QueryPendingFlows`: the pending flows filtered, sorted and paged, with their
/// statistics and categories if asked for.
#[derive(Debug, Clone, Copy)]
pub struct QueryPendingFlows<'a> {
    flows: &'a dyn PendingFlowPort,
}

impl<'a> QueryPendingFlows<'a> {
    pub fn new(flows: &'a dyn PendingFlowPort) -> Self {
        QueryPendingFlows { flows }
    }

    pub fn execute(&self, query: &PendingFlowQuery) -> Result<PendingFlowPage, ReadError> {
        let flows = self.flows.get_all()?;
        let filtered = filter(
            &flows,
            query.statuses.as_deref(),
            query.flow_type,
            query.categories.as_deref(),
        );
        let ordered = sort(filtered, query.sort_by, query.order);
        let total = ordered.len();

        let (entries, has_more) = match (query.page, query.limit) {
            (Some(page), Some(limit)) => {
                let start = (i128::from(page) - 1) * i128::from(limit);
                let end = start + i128::from(limit);
                let (from, to) = python_slice(total, start, end);
                let entries = ordered.get(from..to).unwrap_or_default().to_vec();
                (entries, end < total_i128(total))
            }
            _ => (ordered, false),
        };

        let stats = if query.include_stats {
            Some(build_stats(&flows, query.categories.as_deref())?)
        } else {
            None
        };
        let categories = query.include_categories.then(|| build_categories(&flows));

        Ok(PendingFlowPage {
            entries,
            total,
            page: query.page,
            limit: query.limit,
            has_more,
            stats,
            categories,
        })
    }
}

fn total_i128(total: usize) -> i128 {
    i128::try_from(total).unwrap_or(i128::MAX)
}

/// The bounds of `sequence[start:end]` for a sequence of `len` items, with
/// Python's rules: a negative index counts from the end, and both are clamped.
pub fn python_slice(len: usize, start: i128, end: i128) -> (usize, usize) {
    let length = total_i128(len);
    let clamp = |index: i128| -> usize {
        let index = if index < 0 { index + length } else { index };
        usize::try_from(index.clamp(0, length)).unwrap_or(len)
    };
    let (from, to) = (clamp(start), clamp(end));
    (from, to.max(from))
}

/// `_filter`: an empty list of statuses or categories filters nothing, as `None`.
fn filter(
    flows: &[PendingFlow],
    statuses: Option<&[FlowStatus]>,
    flow_type: Option<FlowType>,
    categories: Option<&[String]>,
) -> Vec<PendingFlow> {
    flows
        .iter()
        .filter(|flow| match statuses {
            Some(statuses) if !statuses.is_empty() => statuses.contains(&flow.status),
            _ => true,
        })
        .filter(|flow| flow_type.is_none_or(|flow_type| flow.flow_type == flow_type))
        .filter(|flow| match categories {
            Some(categories) if !categories.is_empty() => flow
                .category
                .as_ref()
                .is_some_and(|category| categories.contains(category)),
            _ => true,
        })
        .cloned()
        .collect()
}

/// `_sort`: stable, so that flows that compare equal keep their order in both
/// directions, as with `sorted(..., reverse=True)`. By date, the flows without
/// one follow in their order.
fn sort(flows: Vec<PendingFlow>, sort_by: FlowSortField, order: SortOrder) -> Vec<PendingFlow> {
    let descending = order == SortOrder::Desc;
    match sort_by {
        FlowSortField::Date => {
            let (mut with_date, without_date): (Vec<_>, Vec<_>) =
                flows.into_iter().partition(|flow| flow.date.is_some());
            with_date.sort_by(|left, right| {
                let ordering = left.date.cmp(&right.date);
                if descending {
                    ordering.reverse()
                } else {
                    ordering
                }
            });
            with_date.extend(without_date);
            with_date
        }
        FlowSortField::Amount => {
            let mut flows = flows;
            flows.sort_by(|left, right| {
                let ordering = left.amount.cmp(&right.amount);
                if descending {
                    ordering.reverse()
                } else {
                    ordering
                }
            });
            flows
        }
    }
}

/// `_build_stats`: the active flows of the categories asked for, each flow on its
/// own when categories are asked for.
fn build_stats(
    flows: &[PendingFlow],
    categories: Option<&[String]>,
) -> Result<PendingFlowStats, ReadError> {
    let active: Vec<PendingFlow> = flows
        .iter()
        .filter(|flow| flow.status == FlowStatus::Active)
        .cloned()
        .collect();
    let active = filter(&active, None, None, categories);
    let per_entry = categories.is_some_and(|categories| !categories.is_empty());
    Ok(PendingFlowStats {
        earning: flow_type_stat(&active, FlowType::Earning, per_entry)?,
        expense: flow_type_stat(&active, FlowType::Expense, per_entry)?,
    })
}

/// `amounts[currency] = amounts.get(currency, Dezimal(0)) + amount`.
fn add_to(
    amounts: &mut Vec<(String, Dezimal)>,
    currency: &str,
    amount: Dezimal,
) -> Result<(), ReadError> {
    let sum = |previous: Dezimal| {
        (previous + amount)
            .value()
            .map_err(|_| ReadError::Unreadable("sum of the amounts".to_owned()))
    };
    match amounts.iter_mut().find(|(known, _)| known == currency) {
        Some((_, total)) => *total = sum(*total)?,
        None => amounts.push((currency.to_owned(), sum(Dezimal::ZERO)?)),
    }
    Ok(())
}

/// `_flow_type_stat`: the categories in the order they first appear, then the
/// flows counted on their own, those without a category or all with `per_entry`.
fn flow_type_stat(
    flows: &[PendingFlow],
    flow_type: FlowType,
    per_entry: bool,
) -> Result<FlowTypeStat, ReadError> {
    let typed: Vec<&PendingFlow> = flows
        .iter()
        .filter(|flow| flow.flow_type == flow_type)
        .collect();
    let mut totals = Vec::new();
    let mut by_category: Vec<(String, Vec<(String, Dezimal)>)> = Vec::new();
    let mut individual = Vec::new();
    for flow in &typed {
        add_to(&mut totals, &flow.currency, flow.amount)?;
        match flow.category.as_deref() {
            Some(category) if !per_entry && !category.is_empty() => {
                let index = match by_category.iter().position(|(known, _)| known == category) {
                    Some(index) => index,
                    None => {
                        by_category.push((category.to_owned(), Vec::new()));
                        by_category.len() - 1
                    }
                };
                if let Some((_, amounts)) = by_category.get_mut(index) {
                    add_to(amounts, &flow.currency, flow.amount)?;
                }
            }
            _ => individual.push(CategoryStat {
                category: flow.category.clone(),
                amounts: vec![(flow.currency.clone(), flow.amount)],
                name: Some(flow.name.clone()),
            }),
        }
    }
    let mut stats: Vec<CategoryStat> = by_category
        .into_iter()
        .map(|(category, amounts)| CategoryStat {
            category: Some(category),
            amounts,
            name: None,
        })
        .collect();
    stats.extend(individual);
    Ok(FlowTypeStat {
        totals,
        by_category: stats,
        count: typed.len(),
    })
}

/// `_build_categories`: every category of any flow, empty ones left out, in the
/// order they first appear.
fn build_categories(flows: &[PendingFlow]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for flow in flows {
        if let Some(category) = flow.category.as_deref()
            && !category.is_empty()
            && !seen.iter().any(|known| known == category)
        {
            seen.push(category.to_owned());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use core_domain::dates::Date;

    use super::*;

    fn flow(
        name: &str,
        amount: &str,
        flow_type: FlowType,
        category: Option<&str>,
        status: FlowStatus,
        date: Option<&str>,
    ) -> PendingFlow {
        PendingFlow {
            id: name.to_owned(),
            name: name.to_owned(),
            amount: Dezimal::from_python_str(amount).unwrap(),
            currency: if name.ends_with('$') { "USD" } else { "EUR" }.to_owned(),
            flow_type,
            category: category.map(str::to_owned),
            status,
            date: date.map(|date| date.parse::<Date>().unwrap()),
            icon: None,
            status_changed_at: None,
        }
    }

    #[derive(Debug)]
    struct Fixed(Vec<PendingFlow>);

    impl PendingFlowPort for Fixed {
        fn get_all(&self) -> Result<Vec<PendingFlow>, ReadError> {
            Ok(self.0.clone())
        }
    }

    fn flows() -> Fixed {
        use FlowStatus::*;
        use FlowType::*;
        Fixed(vec![
            flow("a", "10", Expense, Some("food"), Active, Some("2026-01-02")),
            flow("b", "10.0", Expense, None, Active, None),
            flow(
                "c",
                "5",
                Earning,
                Some("work"),
                Completed,
                Some("2026-01-01"),
            ),
            flow("d", "20", Expense, Some("food"), Active, Some("2026-01-03")),
            flow("e$", "7", Expense, Some(""), Active, None),
            flow(
                "f$",
                "3",
                Expense,
                Some("food"),
                Disabled,
                Some("2026-01-01"),
            ),
        ])
    }

    fn names(page: &PendingFlowPage) -> Vec<&str> {
        page.entries.iter().map(|flow| flow.name.as_str()).collect()
    }

    fn run(query: PendingFlowQuery) -> PendingFlowPage {
        QueryPendingFlows::new(&flows()).execute(&query).unwrap()
    }

    #[test]
    fn sorts_stably_in_both_directions() {
        let page = run(PendingFlowQuery::default());
        assert_eq!(names(&page), ["d", "a", "b", "e$", "c", "f$"]);
        let page = run(PendingFlowQuery {
            order: SortOrder::Asc,
            ..PendingFlowQuery::default()
        });
        assert_eq!(names(&page), ["f$", "c", "e$", "a", "b", "d"]);
        let page = run(PendingFlowQuery {
            sort_by: FlowSortField::Date,
            ..PendingFlowQuery::default()
        });
        assert_eq!(names(&page), ["d", "a", "c", "f$", "b", "e$"]);
    }

    #[test]
    fn filters_and_pages_like_python_slices() {
        let page = run(PendingFlowQuery {
            statuses: Some(vec![FlowStatus::Active]),
            flow_type: Some(FlowType::Expense),
            page: Some(2),
            limit: Some(2),
            ..PendingFlowQuery::default()
        });
        assert_eq!(names(&page), ["b", "e$"]);
        assert_eq!(page.total, 4);
        assert!(!page.has_more);
        let page = run(PendingFlowQuery {
            categories: Some(vec!["food".to_owned()]),
            page: Some(0),
            limit: Some(2),
            ..PendingFlowQuery::default()
        });
        // `[-2:0]` is empty, but 0 < 3.
        assert!(page.entries.is_empty());
        assert!(page.has_more);
        let page = run(PendingFlowQuery {
            page: Some(-1),
            limit: Some(4),
            ..PendingFlowQuery::default()
        });
        // `[-8:-4]` holds the first two of six.
        assert_eq!(names(&page), ["d", "a"]);
        assert_eq!(python_slice(6, -2, 10), (4, 6));
        assert_eq!(python_slice(6, 5, 2), (5, 5));
        assert_eq!(python_slice(0, i128::from(i64::MIN), 3), (0, 0));
    }

    #[test]
    fn builds_stats_and_categories() {
        let page = run(PendingFlowQuery {
            include_stats: true,
            include_categories: true,
            ..PendingFlowQuery::default()
        });
        assert_eq!(
            page.categories.as_deref(),
            Some(&["food".to_owned(), "work".to_owned()][..])
        );
        let stats = page.stats.unwrap();
        assert_eq!(stats.earning.count, 0);
        assert_eq!(stats.expense.count, 4);
        let total = |amounts: &[(String, Dezimal)], currency: &str| {
            amounts
                .iter()
                .find(|(known, _)| known == currency)
                .map(|(_, amount)| *amount)
        };
        assert_eq!(
            total(&stats.expense.totals, "EUR"),
            Dezimal::from_python_str("40").ok()
        );
        assert_eq!(
            total(&stats.expense.totals, "USD"),
            Dezimal::from_python_str("7").ok()
        );
        let categories: Vec<_> = stats
            .expense
            .by_category
            .iter()
            .map(|stat| (stat.category.as_deref(), stat.name.as_deref()))
            .collect();
        assert_eq!(
            categories,
            [
                (Some("food"), None),
                (None, Some("b")),
                (Some(""), Some("e$"))
            ]
        );
        let page = run(PendingFlowQuery {
            include_stats: true,
            categories: Some(vec!["food".to_owned()]),
            ..PendingFlowQuery::default()
        });
        let names: Vec<_> = page
            .stats
            .unwrap()
            .expense
            .by_category
            .iter()
            .map(|stat| stat.name.clone())
            .collect();
        assert_eq!(names, [Some("a".to_owned()), Some("d".to_owned())]);
    }
}
