// SPDX-License-Identifier: AGPL-3.0-only

//! The system clock, the adapter of the `Clock` port.

use core_app::ports::Clock;
use core_domain::dates::Date;

/// Today's date in the server's local time zone, like Python's `date.today()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn today(&self) -> Date {
        jiff::ZonedDateTime::now().date()
    }
}
