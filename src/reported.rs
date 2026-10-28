//! `Reported<T>`: the difference between a measurement and an absence.
//!
//! A provider that prints `cached: 0` has measured zero. A provider that prints nothing about
//! caching has not measured anything, and treating the absence as zero would invent a cache miss
//! — or a cache share — that nobody observed. [`Reported`] keeps the two apart, and it
//! deliberately has no `Default` and no `Deref`: reaching the value requires saying which case you
//! mean.

/// A value a provider may or may not have reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reported<T> {
    /// The provider did not report the field, so no value exists.
    NotReported,
    /// The provider reported this value; zero is a real measurement.
    Reported(T),
}

impl<T> Reported<T> {
    /// A reported value.
    ///
    /// Named for the variant it builds; the shape reads as `Reported::reported(x)` at a call site
    /// even when the type is inferred.
    #[allow(clippy::self_named_constructors)]
    #[must_use]
    pub const fn reported(value: T) -> Self {
        Reported::Reported(value)
    }

    /// Whether a value exists.
    #[must_use]
    pub const fn is_reported(&self) -> bool {
        matches!(self, Reported::Reported(_))
    }

    /// The value, when one was reported.
    #[must_use]
    pub const fn as_reported(&self) -> Option<&T> {
        match self {
            Reported::NotReported => None,
            Reported::Reported(value) => Some(value),
        }
    }

    /// Transform a reported value, preserving absence.
    #[must_use]
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> Reported<U> {
        match self {
            Reported::NotReported => Reported::NotReported,
            Reported::Reported(value) => Reported::Reported(transform(value)),
        }
    }

    /// Transform a reported value, allowing the transformation itself to answer "not reported".
    #[must_use]
    pub fn and_then<U>(self, transform: impl FnOnce(T) -> Reported<U>) -> Reported<U> {
        match self {
            Reported::NotReported => Reported::NotReported,
            Reported::Reported(value) => transform(value),
        }
    }

    /// The reported value, or a caller-chosen fallback.
    #[must_use]
    pub fn unwrap_or(self, fallback: T) -> T {
        match self {
            Reported::NotReported => fallback,
            Reported::Reported(value) => value,
        }
    }
}

impl<T: Copy> Reported<T> {
    /// The reported value, copied, when one exists.
    #[must_use]
    pub const fn copied(&self) -> Option<T> {
        match self {
            Reported::NotReported => None,
            Reported::Reported(value) => Some(*value),
        }
    }
}

impl<T> From<Option<T>> for Reported<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => Reported::Reported(value),
            None => Reported::NotReported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absence_and_measured_zero_are_different_values() {
        let absent: Reported<u64> = Reported::NotReported;
        let zero = Reported::Reported(0_u64);
        assert_ne!(absent, zero);
        assert!(!absent.is_reported());
        assert!(zero.is_reported());
        assert_eq!(absent.as_reported(), None);
        assert_eq!(zero.as_reported(), Some(&0));
    }

    #[test]
    fn map_preserves_absence_and_never_invents_a_value() {
        let absent: Reported<u64> = Reported::NotReported;
        assert_eq!(absent.map(|value| value + 1), Reported::NotReported);
        assert_eq!(
            Reported::Reported(1_u64).map(|value| value + 1),
            Reported::Reported(2_u64)
        );
        let also_absent: Reported<u64> =
            Reported::Reported(1_u64).and_then(|_| Reported::NotReported);
        assert_eq!(also_absent, Reported::NotReported);
    }

    #[test]
    fn an_option_converts_without_choosing_a_zero() {
        assert_eq!(Reported::<u8>::from(None), Reported::NotReported);
        assert_eq!(Reported::from(Some(0_u8)), Reported::Reported(0_u8));
    }
}
