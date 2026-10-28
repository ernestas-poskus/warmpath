//! The crate's public quantities: counts, ratios, instants, identifiers, and digests.
//!
//! Every type here has a **private field and a validating constructor**. A token count cannot be
//! used where a duration is expected, a ratio cannot have a zero denominator, a basis-point value
//! cannot exceed 100 %, and an identifier cannot be empty. Arithmetic is explicit: checked or
//! saturating methods, never `+` on the inner integer.
//!
//! `Serialize`/`Deserialize` are transparent for scalar quantities and `try_from`-validated for
//! string identifiers, so a wire value can never bypass its constructor.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

macro_rules! count_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            /// The zero value.
            pub const ZERO: Self = Self(0);

            /// Wrap a raw count.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// The raw count.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }

            /// Sum, saturating at [`u64::MAX`].
            #[must_use]
            pub const fn saturating_add(self, other: Self) -> Self {
                Self(self.0.saturating_add(other.0))
            }

            /// Sum, or `None` on overflow.
            #[must_use]
            pub const fn checked_add(self, other: Self) -> Option<Self> {
                match self.0.checked_add(other.0) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// Difference, saturating at zero.
            #[must_use]
            pub const fn saturating_sub(self, other: Self) -> Self {
                Self(self.0.saturating_sub(other.0))
            }

            /// Difference, or `None` when it would go below zero.
            #[must_use]
            pub const fn checked_sub(self, other: Self) -> Option<Self> {
                match self.0.checked_sub(other.0) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// Scale by a basis-point share, rounding half up.
            ///
            /// Wide multiplication keeps the intermediate exact for every `u64` count, so a
            /// large count scaled by 100 % cannot wrap.
            #[must_use]
            pub fn scale(self, share: BasisPoints) -> Self {
                let product = u128::from(self.0) * u128::from(share.get()) + 5_000;
                Self(u64::try_from(product / 10_000).unwrap_or(u64::MAX))
            }

            /// Whether the count is zero.
            #[must_use]
            pub const fn is_zero(self) -> bool {
                self.0 == 0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }
    };
}

count_newtype!(
    /// Input tokens as a provider reports them: the reported denominator before normalization.
    Tokens
);
count_newtype!(
    /// Input tokens that were not served from a cache (fresh input).
    FreshTokens
);
count_newtype!(
    /// Input tokens the provider served from its prompt cache (cache reads).
    CachedTokens
);
count_newtype!(
    /// Input tokens a provider wrote into its cache (cache creation / writes).
    CacheWriteTokens
);
count_newtype!(
    /// Tokens the model produced.
    OutputTokens
);
count_newtype!(
    /// Reasoning tokens, when a provider reports them separately from output.
    ReasoningTokens
);

/// A share in basis points: `0..=10_000`, where 10 000 is 100 %.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct BasisPoints(u16);

impl BasisPoints {
    /// Zero basis points.
    pub const ZERO: Self = Self(0);

    /// 10 000 basis points (100 %).
    pub const MAX: Self = Self(10_000);

    /// Validate a basis-point value.
    #[must_use]
    pub const fn new(value: u16) -> Option<Self> {
        if value <= 10_000 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The raw value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The share as a rounded percentage (`0..=100`).
    #[must_use]
    pub const fn percent(self) -> u8 {
        ((self.0 + 50) / 100) as u8
    }

    /// The share as a fraction in `[0, 1]`.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 10_000.0
    }

    /// The complement (`10_000 - self`).
    #[must_use]
    pub const fn complement(self) -> Self {
        Self(10_000 - self.0)
    }

    /// A share from a ratio, rounded half up, or `None` for a zero denominator.
    #[must_use]
    pub fn from_ratio(numerator: u128, denominator: u128) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let scaled = (numerator.saturating_mul(10_000) + denominator / 2) / denominator;
        let clamped = u16::try_from(scaled.min(10_000)).unwrap_or(10_000);
        Self::new(clamped)
    }
}

impl fmt::Display for BasisPoints {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{:02}%", self.0 / 100, self.0 % 100)
    }
}

impl TryFrom<u16> for BasisPoints {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| Error::Overflow {
            detail: format!("{value} basis points exceeds 10 000"),
        })
    }
}

/// A ratio with a **non-zero denominator** and wide intermediates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ratio {
    numerator: u128,
    denominator: u128,
}

impl Ratio {
    /// Build a ratio; `None` when the denominator is zero.
    ///
    /// The fraction is reduced to lowest terms, so equality and display do not depend on how the
    /// caller spelled it (`2000/1000` and `2/1` are the same ratio).
    #[must_use]
    pub fn new(numerator: u128, denominator: u128) -> Option<Self> {
        if denominator == 0 {
            None
        } else {
            Some(reduced(numerator, denominator))
        }
    }

    /// The numerator.
    #[must_use]
    pub const fn numerator(self) -> u128 {
        self.numerator
    }

    /// The denominator (never zero).
    #[must_use]
    pub const fn denominator(self) -> u128 {
        self.denominator
    }

    /// The ratio as basis points, or `None` when the value exceeds 100 %.
    #[must_use]
    pub fn basis_points(self) -> Option<BasisPoints> {
        let scaled =
            (self.numerator.saturating_mul(10_000) + self.denominator / 2) / self.denominator;
        u16::try_from(scaled).ok().and_then(BasisPoints::new)
    }

    /// The ratio as a fraction in `[0, 1]`.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    /// Subtract another ratio, saturating at zero.
    #[must_use]
    pub fn saturating_sub(self, other: Self) -> Self {
        let common = lcm(self.denominator, other.denominator);
        let left = self.numerator.saturating_mul(common / self.denominator);
        let right = other.numerator.saturating_mul(common / other.denominator);
        reduced(left.saturating_sub(right), common)
    }
}

/// Reduce a fraction to lowest terms (with a zero numerator becoming `0/1`).
fn reduced(numerator: u128, denominator: u128) -> Ratio {
    let divisor = gcd(numerator.max(1), denominator.max(1));
    Ratio {
        numerator: numerator / divisor,
        denominator: denominator / divisor,
    }
}

/// Greatest common divisor.
fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let rest = left % right;
        left = right;
        right = rest;
    }
    left.max(1)
}

/// Least common multiple, saturating at [`u128::MAX`].
fn lcm(left: u128, right: u128) -> u128 {
    let (mut a, mut b) = (left, right);
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    left.saturating_mul(right) / a.max(1)
}

impl fmt::Display for Ratio {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.numerator, self.denominator)
    }
}

macro_rules! instant_newtype {
    ($(#[$meta:meta])* $name:ident, $inner:ty) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            /// The zero value.
            pub const ZERO: Self = Self(0);

            /// Wrap a raw value.
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The raw value.
            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }

            /// Add, saturating at the type's bounds.
            #[must_use]
            pub const fn saturating_add(self, other: $inner) -> Self {
                Self(self.0.saturating_add(other))
            }

            /// Subtract, saturating at the type's bounds.
            #[must_use]
            pub const fn saturating_sub(self, other: $inner) -> Self {
                Self(self.0.saturating_sub(other))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

instant_newtype!(
    /// A Unix epoch second.
    EpochSeconds,
    i64
);
instant_newtype!(
    /// Whole seconds since a warm session ended; an ended-at in the future is zero.
    WarmAgeSecs,
    u64
);

/// A positive lifetime in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TtlSecs(u64);

impl TtlSecs {
    /// A positive TTL; zero is not a lifetime.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// The raw seconds.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The instant `self` seconds after `start`, saturating.
    #[must_use]
    pub fn after(self, start: EpochSeconds) -> EpochSeconds {
        start.saturating_add(i64::try_from(self.0).unwrap_or(i64::MAX))
    }

    /// Seconds remaining at `now`, or zero once expired.
    #[must_use]
    pub fn remaining(self, now: EpochSeconds, start: EpochSeconds) -> WarmAgeSecs {
        let age = warm_age(now, start).get();
        WarmAgeSecs::new(self.0.saturating_sub(age))
    }
}

impl fmt::Display for TtlSecs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}s", self.0)
    }
}

/// Seconds between `now` and `ended_at`; an ended-at in the future counts as zero.
#[must_use]
pub fn warm_age(now: EpochSeconds, ended_at: EpochSeconds) -> WarmAgeSecs {
    WarmAgeSecs::new(u64::try_from(now.get().saturating_sub(ended_at.get())).unwrap_or(0))
}

macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident, $max:expr, $what:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap a value.
            ///
            /// # Errors
            ///
            /// Empty (after trimming), longer than the type's bound, or containing control
            /// characters.
            pub fn new(value: impl Into<String>) -> Result<Self, Error> {
                let value = value.into();
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(Error::InvalidName {
                        kind: $what,
                        value,
                        detail: "must not be empty",
                    });
                }
                if trimmed.len() > $max {
                    return Err(Error::InvalidName {
                        kind: $what,
                        value,
                        detail: "is longer than the type's bound",
                    });
                }
                if trimmed.chars().any(char::is_control) {
                    return Err(Error::InvalidName {
                        kind: $what,
                        value,
                        detail: "must not contain control characters",
                    });
                }
                Ok(Self(trimmed.to_owned()))
            }

            /// The value.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::new(raw)
            }
        }
    };
}

string_newtype!(
    /// The identifier of a metric contract.
    ContractId,
    96,
    "contract id"
);
string_newtype!(
    /// The identifier of a harness profile.
    ProfileId,
    96,
    "profile id"
);
string_newtype!(
    /// A harness name (the external agent CLI).
    HarnessId,
    96,
    "harness id"
);
string_newtype!(
    /// A provider name (the model API family).
    ProviderId,
    96,
    "provider id"
);
string_newtype!(
    /// A model name.
    ModelId,
    160,
    "model id"
);
string_newtype!(
    /// The subject an artifact or a warm record belongs to (an opaque caller-chosen unit of work).
    SubjectId,
    128,
    "subject id"
);
string_newtype!(
    /// A revision of a subject.
    RevisionId,
    128,
    "revision id"
);
string_newtype!(
    /// A harness version string, compared as opaque text.
    HarnessVersion,
    96,
    "harness version"
);

/// A contract's schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContractVersion(u32);

impl ContractVersion {
    /// Wrap a version.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The raw version.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ContractVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A profile's schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileVersion(u16);

impl ProfileVersion {
    /// Wrap a version.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// The raw version.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl fmt::Display for ProfileVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

macro_rules! digest_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate a 64-character lowercase hex digest.
            ///
            /// # Errors
            ///
            /// Wrong length or a non-lowercase-hex character.
            pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
                let value = value.into();
                let trimmed = value.trim();
                if trimmed.len() != 64 {
                    return Err(Error::Malformed {
                        detail: format!("a digest is 64 hex characters, got {}", trimmed.len()),
                    });
                }
                if !trimmed.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
                    return Err(Error::Malformed {
                        detail: "a digest is lowercase hexadecimal".to_owned(),
                    });
                }
                Ok(Self(trimmed.to_owned()))
            }

            /// The digest text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The first eight characters, the short form artifact names use.
            #[must_use]
            pub fn short(&self) -> &str {
                &self.0[..8]
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::parse(raw)
            }
        }
    };
}

digest_newtype!(
    /// The SHA-256 of a canonical prompt prefix.
    PrefixDigest
);
digest_newtype!(
    /// The SHA-256 of an artifact's bytes, or of an identity document.
    ArtifactDigest
);

/// A caller-chosen request identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RequestId(String);

impl RequestId {
    /// Validate and wrap a request id.
    ///
    /// # Errors
    ///
    /// An empty or control-character-containing id is refused.
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
            return Err(Error::InvalidName {
                kind: "request id",
                value,
                detail: "must be non-empty and free of control characters",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl TryFrom<String> for RequestId {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RequestId> for String {
    fn from(value: RequestId) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_points_refuse_more_than_one_hundred_percent() {
        assert!(BasisPoints::new(10_000).is_some());
        assert!(BasisPoints::new(10_001).is_none());
        assert_eq!(BasisPoints::new(9_970).expect("valid").percent(), 100);
        assert_eq!(BasisPoints::from_ratio(1, 2), BasisPoints::new(5_000));
        assert_eq!(
            BasisPoints::new(1_234).expect("valid").to_string(),
            "12.34%"
        );
    }

    #[test]
    fn a_ratio_refuses_a_zero_denominator_and_rounds_half_up() {
        assert!(Ratio::new(1, 0).is_none());
        let ratio = Ratio::new(1, 3).expect("valid");
        assert_eq!(ratio.basis_points(), BasisPoints::new(3_333));
        assert_eq!(
            Ratio::new(2, 4).expect("valid").basis_points(),
            BasisPoints::new(5_000)
        );
    }

    #[test]
    fn counts_saturate_rather_than_wrap() {
        let max = Tokens::new(u64::MAX);
        assert_eq!(max.saturating_add(Tokens::new(1)), max);
        assert!(max.checked_add(Tokens::new(1)).is_none());
        assert_eq!(Tokens::new(3).saturating_sub(Tokens::new(5)), Tokens::ZERO);
        assert_eq!(
            Tokens::new(1_000).scale(BasisPoints::MAX),
            Tokens::new(1_000)
        );
        assert_eq!(
            Tokens::new(1_000).scale(BasisPoints::new(5_000).expect("valid")),
            Tokens::new(500)
        );
    }

    #[test]
    fn identifiers_are_trimmed_and_validated() {
        assert!(HarnessId::new("  codex ").is_ok());
        assert_eq!(HarnessId::new("codex").expect("valid").as_str(), "codex");
        assert!(HarnessId::new("").is_err());
        assert!(HarnessId::new("a\u{0}b").is_err());
        assert!(ProviderId::new("x".repeat(97)).is_err());
    }

    #[test]
    fn digests_are_lowercase_hex_of_exactly_sixty_four_characters() {
        let digest = "a".repeat(64);
        let parsed = PrefixDigest::parse(&digest).expect("valid");
        assert_eq!(parsed.short(), "aaaaaaaa");
        assert!(PrefixDigest::parse("A".repeat(64)).is_err());
        assert!(PrefixDigest::parse("a".repeat(63)).is_err());
    }

    #[test]
    fn a_ttl_is_positive_and_expires_by_whole_seconds() {
        assert!(TtlSecs::new(0).is_none());
        let ttl = TtlSecs::new(300).expect("valid");
        let start = EpochSeconds::new(1_000);
        assert_eq!(ttl.after(start), EpochSeconds::new(1_300));
        assert_eq!(
            warm_age(EpochSeconds::new(1_100), start),
            WarmAgeSecs::new(100)
        );
        assert_eq!(
            warm_age(EpochSeconds::new(900), start),
            WarmAgeSecs::new(0),
            "a future ended-at is zero age"
        );
    }

    #[test]
    fn every_quantity_round_trips_through_serde() {
        let cached = CachedTokens::new(900);
        let encoded = serde_json::to_string(&cached).expect("serializes");
        assert_eq!(encoded, "900");
        assert_eq!(
            serde_json::from_str::<CachedTokens>(&encoded).expect("deserializes"),
            cached
        );
        let harness = HarnessId::new("codex").expect("valid");
        assert_eq!(
            serde_json::to_string(&harness).expect("serializes"),
            "\"codex\""
        );
        assert_eq!(
            serde_json::from_str::<HarnessId>("\"codex\"").expect("deserializes"),
            harness
        );
        assert!(serde_json::from_str::<HarnessId>("\"\"").is_err());
    }
}
