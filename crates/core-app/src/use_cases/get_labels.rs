// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/get_labels.py`.

use core_domain::labeling::Label;

use crate::ports::{LabelPort, PortError};

/// `GetLabels`: every label with the number of transactions that carry it.
#[derive(Debug, Clone, Copy)]
pub struct GetLabels<'a> {
    labels: &'a dyn LabelPort,
}

impl<'a> GetLabels<'a> {
    pub fn new(labels: &'a dyn LabelPort) -> Self {
        GetLabels { labels }
    }

    /// The labels in the port's order, each with its usage, zero for a label no
    /// transaction carries.
    pub fn execute(&self) -> Result<Vec<Label>, PortError> {
        let mut labels = self.labels.get_all()?;
        let usage = self.labels.get_usage()?;
        for label in &mut labels {
            label.usage = Some(usage.get(&label.id).copied().unwrap_or(0));
        }
        Ok(labels)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use core_domain::labeling::{LabelCategory, LabelSubcategory};

    use super::*;

    #[derive(Debug)]
    struct Fixed;

    fn label(id: &str) -> Label {
        Label {
            id: id.to_owned(),
            key: None,
            name: Some(id.to_owned()),
            description: None,
            color: None,
            icon: None,
            category: LabelCategory::Expense,
            subcategory: LabelSubcategory::Need,
            usage: None,
        }
    }

    impl LabelPort for Fixed {
        fn get_all(&self) -> Result<Vec<Label>, PortError> {
            Ok(vec![label("a"), label("b")])
        }

        fn get_usage(&self) -> Result<HashMap<String, u64>, PortError> {
            Ok(HashMap::from([("b".to_owned(), 3), ("c".to_owned(), 1)]))
        }
    }

    #[test]
    fn counts_the_usage_of_every_label() {
        let labels = GetLabels::new(&Fixed).execute().unwrap();
        let usage: Vec<_> = labels
            .iter()
            .map(|label| (label.id.as_str(), label.usage))
            .collect();
        assert_eq!(usage, [("a", Some(0)), ("b", Some(3))]);
    }
}
