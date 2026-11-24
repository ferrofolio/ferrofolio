// SPDX-License-Identifier: AGPL-3.0-only

//! Enums ported from Python's `class Name(str, Enum)`.
//!
//! Every member of such a class is a string: its value is what the API, the database
//! and the JSON files carry, and Python compares members by that value. The
//! [`str_enum!`] macro gives a Rust enum the same values, in the same order, with
//! [`StrEnum`] and inherent methods of the same names.

/// An enum ported from a Python `(str, Enum)` class.
pub trait StrEnum: Copy + Sized + 'static {
    /// The name of the Python class, as Python's error messages print it.
    const NAME: &'static str;
    /// Every member, in the order of the Python class.
    const ALL: &'static [Self];

    /// The member's value.
    fn as_str(self) -> &'static str;

    /// `Name(value)` in Python: the member with exactly this value.
    fn from_value(value: &str) -> Option<Self>;
}

/// Declares an enum ported from a Python `(str, Enum)` class.
///
/// Each variant is written with the value of its Python member. The enum gets
/// `NAME`, `ALL`, `as_str` and `from_value` as inherent items and through
/// [`StrEnum`], `Display` with the value, and an order by value, as Python orders
/// members of a `str` enum. A default member is marked with `#[default]` and
/// `#[derive(Default)]` on the enum.
macro_rules! str_enum {
    (
        $(#[$attribute:meta])*
        $visibility:vis enum $name:ident {
            $(
                $(#[$variant_attribute:meta])*
                $variant:ident = $value:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$attribute])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $visibility enum $name {
            $(
                $(#[$variant_attribute])*
                $variant,
            )+
        }

        impl $name {
            /// The name of the Python class.
            pub const NAME: &'static str = stringify!($name);
            /// Every member, in the order of the Python class.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The member's value, which the API and the database carry.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $value,)+
                }
            }

            /// `Name(value)` in Python: the member with exactly this value.
            pub fn from_value(value: &str) -> Option<Self> {
                match value {
                    $($value => Some($name::$variant),)+
                    _ => None,
                }
            }
        }

        impl $crate::str_enum::StrEnum for $name {
            const NAME: &'static str = $name::NAME;
            const ALL: &'static [$name] = $name::ALL;

            fn as_str(self) -> &'static str {
                $name::as_str(self)
            }

            fn from_value(value: &str) -> Option<Self> {
                $name::from_value(value)
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::cmp::PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(::std::cmp::Ord::cmp(self, other))
            }
        }

        impl ::std::cmp::Ord for $name {
            /// Compares the values, as Python compares members of a `str` enum.
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                self.as_str().cmp(other.as_str())
            }
        }
    };
}

pub(crate) use str_enum;

#[cfg(test)]
mod tests {
    use super::StrEnum;

    str_enum! {
        #[derive(Default)]
        enum Example {
            Second = "B",
            #[default]
            First = "A",
        }
    }

    #[test]
    fn members_keep_python_values_and_order() {
        assert_eq!(Example::NAME, "Example");
        assert_eq!(Example::ALL, &[Example::Second, Example::First]);
        assert_eq!(Example::Second.as_str(), "B");
        assert_eq!(Example::from_value("A"), Some(Example::First));
        assert_eq!(Example::from_value("a"), None);
        assert_eq!(<Example as StrEnum>::from_value("B"), Some(Example::Second));
        assert_eq!(Example::default(), Example::First);
        assert_eq!(Example::First.to_string(), "A");
    }

    #[test]
    fn members_order_by_value() {
        // Declared second but "A" < "B", as in Python.
        assert!(Example::First < Example::Second);
        let mut members = Example::ALL.to_vec();
        members.sort();
        assert_eq!(members, vec![Example::First, Example::Second]);
    }
}
