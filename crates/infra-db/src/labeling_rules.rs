// SPDX-License-Identifier: AGPL-3.0-only

//! The labeling rule repository, the reading half of
//! `finanze/infrastructure/repository/labeling/labeling_rule_repository.py`.
//!
//! Python reads a rule's stored values through `json.loads`, `Dezimal`,
//! `date.fromisoformat` and pydantic, which coerce many forms its own writer,
//! `serialize_conditions`, never produces. Rust reads exactly the forms the
//! writer produces, and those of the migrations, and reports any other as
//! [`ReadError::Unreadable`], upon which the route passes the request on to
//! Python. Rust therefore never answers with a rule Python would read
//! differently, nor reproduces Python's errors for data it could not have written.

use std::collections::HashMap;

use core_app::ports::{LabelingRulePort, PortError, ReadError};
use core_domain::Dezimal;
use core_domain::dates::Date;
use core_domain::labeling::{
    LabelingRule, LabelingRuleConditions, LabelingRuleKind, TextCondition, TextMatchField,
    TextMatchOperator,
};
use core_domain::pyuuid;
use core_domain::transactions::TxType;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};

use crate::Database;

/// `LabelingRuleQueries.GET_ALL`.
const GET_ALL: &str = "
    SELECT r.id, r.name, r.enabled, r.kind, r.conditions, rl.label_id
    FROM labeling_rules r
        LEFT JOIN labeling_rule_labels rl ON rl.rule_id = r.id
    ORDER BY r.created_at, r.id
";

/// The key serde_json uses for a number with `arbitrary_precision`; an object with
/// it as its first key is read as a number, which Python reads as a `dict`.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// `LabelingRuleRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct LabelingRuleRepository<'a> {
    database: &'a Database,
}

impl<'a> LabelingRuleRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        LabelingRuleRepository { database }
    }
}

type Read<T> = Result<T, String>;

fn unreadable<T>(what: &str) -> Read<T> {
    Err(what.to_owned())
}

/// A text column, `None` for NULL. Any other type is unreadable.
fn text<'v>(value: ValueRef<'v>, what: &str) -> Read<Option<&'v str>> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => Ok(Some(text)),
            Err(_) => unreadable(what),
        },
        _ => unreadable(what),
    }
}

/// `UUID(value)` of a text column.
fn uuid(value: &str, what: &str) -> Read<String> {
    pyuuid::parse(value).or_else(|_| unreadable(what))
}

/// `bool(value)`, Python's truth value of what `sqlite3` returns.
fn truthy(value: ValueRef<'_>) -> bool {
    match value {
        ValueRef::Null => false,
        ValueRef::Integer(number) => number != 0,
        ValueRef::Real(number) => number != 0.0,
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => !bytes.is_empty(),
    }
}

/// Whether a JSON text has a string, key or value, that decodes to serde_json's
/// number token, however it is escaped.
fn mentions_number_token(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while let Some(offset) = bytes
        .get(index..)
        .and_then(|rest| rest.iter().position(|b| *b == b'"'))
    {
        let start = index + offset;
        let mut end = start + 1;
        let mut escaped = false;
        while let Some(&byte) = bytes.get(end) {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                break;
            }
            end += 1;
        }
        let literal = raw.get(start..=end).unwrap_or_default();
        if serde_json::from_str::<String>(literal).is_ok_and(|decoded| decoded == NUMBER_TOKEN) {
            return true;
        }
        index = end + 1;
    }
    false
}

/// An optional list `if data.get(key)` reads: absent, `null` and `[]` are `None`.
fn list<'v>(group: &'v Map<String, Value>, key: &str) -> Read<Option<&'v Vec<Value>>> {
    match group.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.is_empty() => Ok(None),
        Some(Value::Array(items)) => Ok(Some(items)),
        Some(_) => unreadable(key),
    }
}

/// A list of strings, each read by `read`.
fn strings<T>(
    group: &Map<String, Value>,
    key: &str,
    read: impl Fn(&str) -> Option<T>,
) -> Read<Option<Vec<T>>> {
    let Some(items) = list(group, key)? else {
        return Ok(None);
    };
    items
        .iter()
        .map(|item| item.as_str().and_then(&read).ok_or_else(|| key.to_owned()))
        .collect::<Read<Vec<T>>>()
        .map(Some)
}

/// `Dezimal(data[key]) if data.get(key) is not None`, for the string the writer
/// stores. A number that a float cannot hold, which Python would answer as
/// `Infinity`, is unreadable too.
fn amount(group: &Map<String, Value>, key: &str) -> Read<Option<Dezimal>> {
    match group.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => match Dezimal::from_python_str(text) {
            Ok(amount) if amount.to_f64_for_api().is_finite() => Ok(Some(amount)),
            _ => unreadable(key),
        },
        Some(_) => unreadable(key),
    }
}

/// `data.get(key)` into `Optional[str]`.
fn optional_string(group: &Map<String, Value>, key: &str) -> Read<Option<String>> {
    match group.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => unreadable(key),
    }
}

/// `data.get(key)` into `Optional[int]`, for the integers the writer stores.
fn integer(group: &Map<String, Value>, key: &str) -> Read<Option<i64>> {
    match group.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number.as_i64().map(Some).ok_or_else(|| key.to_owned()),
        Some(_) => unreadable(key),
    }
}

/// `date.fromisoformat(data[key]) if data.get(key)`, for `YYYY-MM-DD`, the form
/// `date.isoformat()` writes.
fn date(group: &Map<String, Value>, key: &str) -> Read<Option<Date>> {
    let text = match group.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(text)) if text.is_empty() => return Ok(None),
        Some(Value::String(text)) => text,
        Some(_) => return unreadable(key),
    };
    crate::read::iso_date(text)
        .map(Some)
        .ok_or_else(|| key.to_owned())
}

/// The text condition, `None` when `text` is absent, `null` or `{}`.
fn text_condition(group: &Map<String, Value>) -> Read<Option<TextCondition>> {
    let text = match group.get("text") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(text)) if text.is_empty() => return Ok(None),
        Some(Value::Object(text)) => text,
        Some(_) => return unreadable("text"),
    };
    let values = match text.get("values") {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "text values".to_owned())
            })
            .collect::<Read<Vec<String>>>()?,
        _ => return unreadable("text values"),
    };
    let operator = match text.get("operator") {
        None => TextMatchOperator::Contains,
        Some(value) => value
            .as_str()
            .and_then(TextMatchOperator::parse)
            .ok_or_else(|| "text operator".to_owned())?,
    };
    let field = match text.get("field") {
        None => TextMatchField::Any,
        Some(value) => value
            .as_str()
            .and_then(TextMatchField::parse)
            .ok_or_else(|| "text field".to_owned())?,
    };
    Ok(Some(TextCondition {
        values,
        operator,
        field,
    }))
}

/// `_deserialize_group`. Keys the reader does not know are ignored, as in Python.
fn group(value: &Value) -> Read<LabelingRuleConditions> {
    let Value::Object(group) = value else {
        return unreadable("condition group");
    };
    Ok(LabelingRuleConditions {
        types: strings(group, "types", TxType::parse)?,
        min_amount: amount(group, "min_amount")?,
        max_amount: amount(group, "max_amount")?,
        currency: optional_string(group, "currency")?,
        from_date: date(group, "from_date")?,
        to_date: date(group, "to_date")?,
        day_from: integer(group, "day_from")?,
        day_to: integer(group, "day_to")?,
        entities: strings(group, "entities", |entity| pyuuid::parse(entity).ok())?,
        text: text_condition(group)?,
        max_days: integer(group, "max_days")?,
        ibans: strings(group, "ibans", |iban| Some(iban.to_owned()))?,
    })
}

/// `deserialize_conditions`: a JSON list of groups, `[]` for an empty text.
pub fn deserialize_conditions(raw: &str) -> Read<Vec<LabelingRuleConditions>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    if mentions_number_token(raw) {
        return unreadable("conditions");
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(groups)) => groups.iter().map(group).collect(),
        _ => unreadable("conditions"),
    }
}

/// One row of the query: the rule's columns and one of its labels.
struct Row {
    id: String,
    name: Option<String>,
    enabled: bool,
    kind: String,
    conditions: String,
    label_id: Option<String>,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Read<Row>> {
    let mut columns = Vec::with_capacity(6);
    for index in 0..6 {
        columns.push(row.get_ref(index)?);
    }
    let column = |index: usize| columns.get(index).copied().unwrap_or(ValueRef::Null);
    let read = || -> Read<Row> {
        let id = text(column(0), "id")?.ok_or_else(|| "id".to_owned())?;
        let kind = text(column(3), "kind")?.ok_or_else(|| "kind".to_owned())?;
        // `json.loads(raw) if raw else []`; the column is never NULL in the schema.
        let conditions = text(column(4), "conditions")?.ok_or_else(|| "conditions".to_owned())?;
        // `if row["label_id"]`: NULL and the empty text add no label.
        let label_id = match column(5) {
            ValueRef::Null => None,
            other => text(other, "label id")?
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
        };
        Ok(Row {
            id: id.to_owned(),
            name: text(column(1), "name")?.map(str::to_owned),
            enabled: truthy(column(2)),
            kind: kind.to_owned(),
            conditions: conditions.to_owned(),
            label_id,
        })
    };
    Ok(read())
}

/// `_group_rows`: one rule per stored id, in the order of its first row, built
/// from that row, with the labels of all its rows.
fn group_rows(rows: Vec<Row>) -> Read<Vec<LabelingRule>> {
    let mut rules: Vec<LabelingRule> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let position = match index.get(&row.id) {
            Some(&position) => position,
            None => {
                let rule = LabelingRule {
                    id: uuid(&row.id, "id")?,
                    name: row.name,
                    enabled: row.enabled,
                    kind: LabelingRuleKind::parse(&row.kind).ok_or_else(|| "kind".to_owned())?,
                    conditions: deserialize_conditions(&row.conditions)?,
                    label_ids: Vec::new(),
                };
                rules.push(rule);
                index.insert(row.id, rules.len() - 1);
                rules.len() - 1
            }
        };
        if let Some(label_id) = row.label_id {
            let label_id = uuid(&label_id, "label id")?;
            if let Some(rule) = rules.get_mut(position) {
                rule.label_ids.push(label_id);
            }
        }
    }
    Ok(rules)
}

impl LabelingRulePort for LabelingRuleRepository<'_> {
    fn get_all(&self) -> Result<Vec<LabelingRule>, ReadError> {
        let sqlite = |error: rusqlite::Error| ReadError::Failed(PortError(error.to_string()));
        let mut statement = self.database.connection.prepare(GET_ALL).map_err(sqlite)?;
        let rows = statement.query_map([], read_row).map_err(sqlite)?;
        let mut read = Vec::new();
        for row in rows {
            read.push(row.map_err(sqlite)?.map_err(ReadError::Unreadable)?);
        }
        group_rows(read).map_err(ReadError::Unreadable)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use rusqlite::types::Value as Sql;

    use super::*;

    const ID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const LABEL: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

    fn database(rules: &[(Sql, Sql, Sql, Sql, Sql)], labels: &[(Sql, Sql)]) -> Database {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE labeling_rules (id CHAR(36), name TEXT, enabled BOOLEAN,
                 kind VARCHAR(16), conditions TEXT, created_at DATETIME, updated_at DATETIME);
                 CREATE TABLE labeling_rule_labels (rule_id CHAR(36), label_id CHAR(36),
                 PRIMARY KEY (rule_id, label_id));",
            )
            .unwrap();
        for (index, (id, name, enabled, kind, conditions)) in rules.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO labeling_rules VALUES (?, ?, ?, ?, ?, ?, '')",
                    rusqlite::params![id, name, enabled, kind, conditions, index.to_string()],
                )
                .unwrap();
        }
        for (rule, label) in labels {
            connection
                .execute(
                    "INSERT INTO labeling_rule_labels VALUES (?, ?)",
                    rusqlite::params![rule, label],
                )
                .unwrap();
        }
        Database { connection }
    }

    fn t(value: &str) -> Sql {
        Sql::Text(value.to_owned())
    }

    fn rule(conditions: &str) -> (Sql, Sql, Sql, Sql, Sql) {
        (t(ID), Sql::Null, Sql::Integer(1), t("MATCH"), t(conditions))
    }

    fn read(database: &Database) -> Result<Vec<LabelingRule>, ReadError> {
        LabelingRuleRepository::new(database).get_all()
    }

    #[test]
    fn reads_what_the_writer_stores() {
        let database = database(
            &[rule(
                r#"[{"types": ["INFLOW"], "min_amount": "1.50", "currency": "EUR",
                     "from_date": "2026-01-31", "day_from": 3, "entities": ["E0000000000000000000000000000001"],
                     "text": {"values": ["Caf\u00e9"], "operator": "REGEX", "field": "NAME"},
                     "ibans": ["XX00TEST"]}, {"max_days": 3}]"#,
            )],
            &[(t(ID), t(&LABEL.to_uppercase())), (t(ID), t(""))],
        );
        let rules = read(&database).unwrap();
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.label_ids, [LABEL]);
        let first = &rule.conditions[0];
        assert_eq!(first.types, Some(vec![TxType::Inflow]));
        assert_eq!(
            first.min_amount,
            Some(Dezimal::from_python_str("1.5").unwrap())
        );
        assert_eq!(first.from_date, Some(Date::new(2026, 1, 31).unwrap()));
        assert_eq!(first.day_from, Some(3));
        assert_eq!(
            first.entities,
            Some(vec!["e0000000-0000-0000-0000-000000000001".to_owned()])
        );
        let text = first.text.as_ref().unwrap();
        assert_eq!(text.values, ["Café"]);
        assert_eq!(text.operator, TextMatchOperator::Regex);
        assert_eq!(rule.conditions[1].max_days, Some(3));
    }

    #[test]
    fn reads_falsy_values_as_python_does() {
        let groups = deserialize_conditions(
            r#"[{"types": [], "entities": null, "text": {}, "from_date": "", "ibans": [], "other": 1}]"#,
        )
        .unwrap();
        assert_eq!(groups, [LabelingRuleConditions::default()]);
        assert_eq!(deserialize_conditions("").unwrap(), []);
        let text = deserialize_conditions(r#"[{"text": {"values": []}}]"#).unwrap();
        let text = text[0].text.as_ref().unwrap();
        assert_eq!(text.operator, TextMatchOperator::Contains);
        assert_eq!(text.field, TextMatchField::Any);
    }

    #[test]
    fn leaves_every_other_form_to_python() {
        for raw in [
            "{}",
            "[1]",
            "NaN",
            "[{\"day_from\": 3.0}]",
            "[{\"day_from\": \"3\"}]",
            "[{\"day_from\": true}]",
            "[{\"day_from\": 99999999999999999999}]",
            "[{\"day_from\": {\"$serde_json::private::Number\": \"3\"}}]",
            "[{\"day_from\": {\"\\u0024serde_json::private::Number\": \"3\"}}]",
            "[{\"min_amount\": 1}]",
            "[{\"min_amount\": \"NaN\"}]",
            "[{\"min_amount\": \"1e400\"}]",
            "[{\"min_amount\": \"\"}]",
            "[{\"types\": [\"inflow\"]}]",
            "[{\"types\": \"INFLOW\"}]",
            "[{\"entities\": [\"x\"]}]",
            "[{\"from_date\": \"20260101\"}]",
            "[{\"from_date\": \"0000-01-01\"}]",
            "[{\"from_date\": \"2026-02-30\"}]",
            "[{\"from_date\": \"２０２６-01-01\"}]",
            "[{\"text\": {\"operator\": \"REGEX\"}}]",
            "[{\"text\": {\"values\": \"abc\"}}]",
            "[{\"text\": {\"values\": [1]}}]",
            "[{\"text\": {\"values\": [], \"operator\": null}}]",
            "[{\"text\": \"abc\"}]",
            "[{\"currency\": 1}]",
            "[{\"ibans\": [1]}]",
        ] {
            assert!(deserialize_conditions(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn leaves_unreadable_rows_to_python() {
        let cases = [
            (
                t("not-a-uuid"),
                Sql::Null,
                Sql::Integer(1),
                t("MATCH"),
                t("[]"),
            ),
            // Text affinity stores a number as text; a blob reaches Python as bytes.
            (
                t(ID),
                Sql::Blob(b"n".to_vec()),
                Sql::Integer(1),
                t("MATCH"),
                t("[]"),
            ),
            (t(ID), Sql::Null, Sql::Integer(1), t("match"), t("[]")),
            (
                t(ID),
                Sql::Null,
                Sql::Integer(1),
                t("MATCH"),
                Sql::Blob(b"[]".to_vec()),
            ),
            (
                Sql::Blob(ID.as_bytes().to_vec()),
                Sql::Null,
                Sql::Integer(1),
                t("MATCH"),
                t("[]"),
            ),
        ];
        for case in cases {
            let database = database(&[case], &[]);
            assert!(matches!(read(&database), Err(ReadError::Unreadable(_))));
        }
        let database = database(&[rule("[]")], &[(t(ID), t("label"))]);
        assert!(matches!(read(&database), Err(ReadError::Unreadable(_))));
    }

    #[test]
    fn reads_enabled_as_python_truth() {
        for (value, enabled) in [
            (Sql::Integer(0), false),
            (Sql::Integer(2), true),
            (Sql::Null, false),
            (t(""), false),
            // Numeric affinity stores "0" as the integer 0, which Python reads too.
            (t("0"), false),
            (t("x"), true),
            (Sql::Real(0.0), false),
        ] {
            let database = database(&[(t(ID), Sql::Null, value, t("TRANSFER"), t("[]"))], &[]);
            assert_eq!(read(&database).unwrap()[0].enabled, enabled);
        }
    }

    #[test]
    fn keeps_rows_of_differently_spelled_ids_apart() {
        let database = database(
            &[
                rule("[]"),
                (
                    t(&ID.to_uppercase()),
                    Sql::Null,
                    Sql::Integer(1),
                    t("MATCH"),
                    t("[]"),
                ),
            ],
            &[],
        );
        let rules = read(&database).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, rules[1].id);
    }
}
