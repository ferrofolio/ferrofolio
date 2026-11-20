// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/get_labeling_rules.py`.

use core_domain::labeling::LabelingRule;

use crate::ports::{LabelingRulePort, ReadError};

/// `GetLabelingRules`: every rule, enabled or not.
#[derive(Debug, Clone, Copy)]
pub struct GetLabelingRules<'a> {
    rules: &'a dyn LabelingRulePort,
}

impl<'a> GetLabelingRules<'a> {
    pub fn new(rules: &'a dyn LabelingRulePort) -> Self {
        GetLabelingRules { rules }
    }

    /// The rules in the port's order.
    pub fn execute(&self) -> Result<Vec<LabelingRule>, ReadError> {
        self.rules.get_all()
    }
}

#[cfg(test)]
mod tests {
    use core_domain::labeling::{LabelingRuleConditions, LabelingRuleKind};

    use super::*;

    #[derive(Debug)]
    struct Fixed(bool);

    impl LabelingRulePort for Fixed {
        fn get_all(&self) -> Result<Vec<LabelingRule>, ReadError> {
            if !self.0 {
                return Err(ReadError::Unreadable("conditions".to_owned()));
            }
            Ok(vec![LabelingRule {
                id: "a".to_owned(),
                conditions: vec![LabelingRuleConditions::default()],
                label_ids: vec!["b".to_owned()],
                name: None,
                enabled: false,
                kind: LabelingRuleKind::Transfer,
            }])
        }
    }

    #[test]
    fn returns_every_rule_including_disabled_ones() {
        let rules = GetLabelingRules::new(&Fixed(true)).execute().unwrap();
        assert_eq!(rules.len(), 1);
        assert!(!rules[0].enabled);
    }

    #[test]
    fn passes_an_unreadable_rule_on() {
        let error = GetLabelingRules::new(&Fixed(false)).execute().unwrap_err();
        assert_eq!(error, ReadError::Unreadable("conditions".to_owned()));
    }
}
