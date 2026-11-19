// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/get_settings.py`.

use std::collections::BTreeSet;

use core_domain::settings::{PartialSettings, SettingsSection};

use crate::ports::{ConfigPort, PortError};

/// `GetSettings`: reads the sections asked for from the stored settings.
#[derive(Debug, Clone, Copy)]
pub struct GetSettings<'a> {
    config: &'a dyn ConfigPort,
}

impl<'a> GetSettings<'a> {
    pub fn new(config: &'a dyn ConfigPort) -> Self {
        GetSettings { config }
    }

    /// The sections asked for, or every section when none is.
    pub fn execute(
        &self,
        sections: &BTreeSet<SettingsSection>,
    ) -> Result<PartialSettings, PortError> {
        if sections.is_empty() {
            let every: BTreeSet<SettingsSection> = SettingsSection::ALL.iter().copied().collect();
            return self.config.load_sections(&every);
        }
        self.config.load_sections(sections)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default)]
    struct Recording(RefCell<Vec<BTreeSet<SettingsSection>>>);

    impl ConfigPort for Recording {
        fn load_sections(
            &self,
            sections: &BTreeSet<SettingsSection>,
        ) -> Result<PartialSettings, PortError> {
            self.0.borrow_mut().push(sections.clone());
            Ok(PartialSettings::new("then".to_owned()))
        }
    }

    #[test]
    fn asks_for_every_section_when_none_is_given() {
        let port = Recording::default();
        let use_case = GetSettings::new(&port);
        use_case.execute(&BTreeSet::new()).unwrap();
        let one = BTreeSet::from([SettingsSection::Ai]);
        use_case.execute(&one).unwrap();
        let asked = port.0.borrow();
        assert_eq!(asked[0].len(), SettingsSection::ALL.len());
        assert_eq!(asked[1], one);
    }
}
