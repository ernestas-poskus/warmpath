//! Metric contracts: the named rule, its exclusions, and the verdict it supports.
//!
//! A cache metric is only meaningful with its denominator and exclusions written down. This module
//! ships two contracts as **data**:
//!
//! - **AICS** — *agent input cache share*: token-weighted `cached / (cached + fresh)` pooled over
//!   every measured request, cold requests included, output excluded. It answers "how much of the
//!   prompt traffic did the provider serve from cache", not "how good was one run".
//! - **EEPR** — *expected-prefix reuse*: token-weighted `served / expected` over **warm** runs only;
//!   cold runs are excluded from the ratio and reported separately, because a cold start cannot
//!   reuse a prefix that does not exist.
//!
//! Both refuse to produce a number from a zero denominator: an unmeasured gate is `Insufficient`,
//! never a synthetic 0 %. Ratios are [`BasisPoints`], computed with `u128` intermediates and rounded
//! half up, so a very large token count cannot wrap and a measured zero is `Some(0)`.

use std::fmt;

use crate::error::Error;
use crate::newtypes::{
    BasisPoints, CacheWriteTokens, CachedTokens, ContractId, ContractVersion, FreshTokens, Ratio,
    Tokens,
};
use crate::reported::Reported;

/// The unit a contract's verdict is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricUnit {
    /// A share in basis points.
    BasisPoints,
    /// A token count.
    Tokens,
}

/// How a contract's ratio is rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rounding {
    /// Round half up to the nearest basis point.
    HalfUp,
    /// Truncate toward zero.
    Truncate,
}

/// A named metric rule, versioned as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricContract {
    /// The contract id.
    pub id: ContractId,
    /// The contract version.
    pub version: ContractVersion,
    /// A human-readable name.
    pub name: &'static str,
    /// The numerator rule, in words.
    pub numerator: &'static str,
    /// The denominator rule, in words.
    pub denominator: &'static str,
    /// What the rule excludes, exported so a report cannot hide it.
    pub exclusions: &'static [&'static str],
    /// The unit of the verdict.
    pub unit: MetricUnit,
    /// The rounding rule.
    pub rounding: Rounding,
}

impl MetricContract {
    /// The stable code `${id}.v${version}`.
    #[must_use]
    pub fn code(&self) -> String {
        format!("{}.v{}", self.id, self.version)
    }
}

/// The version this build's contracts use.
pub const CONTRACT_VERSION: ContractVersion = ContractVersion::new(1);

/// The agent input cache-share contract.
///
/// # Panics
///
/// Never: the contract id is a static valid literal.
#[must_use]
pub fn aics_contract() -> MetricContract {
    MetricContract {
        id: ContractId::new("aics").expect("static contract id"),
        version: CONTRACT_VERSION,
        name: "agent input cache share",
        numerator: "cache-read input tokens",
        denominator: "cache-read input tokens + fresh input tokens",
        exclusions: &[
            "output tokens",
            "requests whose cache-read count was not reported",
            "unclassified cold/warm status (the ratio includes both)",
        ],
        unit: MetricUnit::BasisPoints,
        rounding: Rounding::HalfUp,
    }
}

/// The expected-prefix reuse contract (warm runs only).
///
/// # Panics
///
/// Never: the contract id is a static valid literal.
#[must_use]
pub fn eepr_contract() -> MetricContract {
    MetricContract {
        id: ContractId::new("eepr").expect("static contract id"),
        version: CONTRACT_VERSION,
        name: "expected-prefix reuse",
        numerator: "cache-read input tokens of warm runs",
        denominator: "expected prompt tokens of warm runs (read + fresh)",
        exclusions: &[
            "cold runs (reported separately)",
            "requests whose expected total could not be established",
            "output tokens",
        ],
        unit: MetricUnit::BasisPoints,
        rounding: Rounding::HalfUp,
    }
}

/// Both built-in contracts.
#[must_use]
pub fn contracts() -> [MetricContract; 2] {
    [aics_contract(), eepr_contract()]
}

/// A verdict from a contract and a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The measured value met the target.
    Pass,
    /// The measured value is below the target.
    Fail,
    /// The measurements cannot support a verdict.
    Insufficient,
}

impl Verdict {
    /// The verdict for a measured value against a target.
    #[must_use]
    pub fn of(measured: Option<BasisPoints>, target: BasisPoints) -> Self {
        match measured {
            None => Verdict::Insufficient,
            Some(value) if value >= target => Verdict::Pass,
            Some(_) => Verdict::Fail,
        }
    }

    /// Whether the verdict is a pass.
    #[must_use]
    pub const fn is_pass(self) -> bool {
        matches!(self, Verdict::Pass)
    }

    /// The stable spelling.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::Insufficient => "insufficient_data",
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code())
    }
}

/// One measured request (or run) for the AICS contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSample {
    /// Cache-read tokens, when reported.
    pub cached: Reported<CachedTokens>,
    /// Fresh input tokens, when reported.
    pub fresh: Reported<FreshTokens>,
    /// Cache-write tokens, when reported; used by the amplification report, never the share.
    pub write: Reported<CacheWriteTokens>,
}

impl CacheSample {
    /// A fully measured sample.
    #[must_use]
    pub fn measured(cached: CachedTokens, fresh: FreshTokens) -> Self {
        Self {
            cached: Reported::Reported(cached),
            fresh: Reported::Reported(fresh),
            write: Reported::NotReported,
        }
    }

    /// A sample that also reports a cache write.
    #[must_use]
    pub fn with_write(mut self, write: CacheWriteTokens) -> Self {
        self.write = Reported::Reported(write);
        self
    }
}

/// The distribution of per-request cache shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestDistribution {
    /// Measured requests.
    pub count: u64,
    /// The minimum share, when any request was measured.
    pub min: Option<BasisPoints>,
    /// The median (nearest-rank).
    pub p50: Option<BasisPoints>,
    /// The 90th percentile (nearest-rank).
    pub p90: Option<BasisPoints>,
    /// The maximum share.
    pub max: Option<BasisPoints>,
}

impl RequestDistribution {
    /// An empty distribution.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            count: 0,
            min: None,
            p50: None,
            p90: None,
            max: None,
        }
    }

    /// Build from per-request shares (any order).
    #[must_use]
    pub fn of(shares: &[BasisPoints]) -> Self {
        if shares.is_empty() {
            return Self::empty();
        }
        let mut sorted = shares.to_vec();
        sorted.sort_unstable();
        let nearest = |percent: usize| -> BasisPoints {
            let index = ((sorted.len() - 1) * percent).div_ceil(100);
            sorted[index.min(sorted.len() - 1)]
        };
        Self {
            count: sorted.len() as u64,
            min: sorted.first().copied(),
            p50: Some(nearest(50)),
            p90: Some(nearest(90)),
            max: sorted.last().copied(),
        }
    }
}

/// A cache-write amplification observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAmplification {
    /// Tokens written into the cache.
    pub writes: CacheWriteTokens,
    /// Tokens served from the cache.
    pub reads: CachedTokens,
    /// Fresh input tokens.
    pub fresh: FreshTokens,
    /// `writes / reads`, when any read was served.
    pub write_read_ratio: Option<Ratio>,
    /// `writes / (writes + reads + fresh)`: how much of prompt work was cache churn.
    pub churn: Option<Ratio>,
}

/// Cache-write amplification and churn from measured totals.
#[must_use]
pub fn write_amplification(
    writes: CacheWriteTokens,
    reads: CachedTokens,
    fresh: FreshTokens,
) -> WriteAmplification {
    WriteAmplification {
        writes,
        reads,
        fresh,
        write_read_ratio: Ratio::new(u128::from(writes.get()), u128::from(reads.get())),
        churn: Ratio::new(
            u128::from(writes.get()),
            u128::from(writes.get()) + u128::from(reads.get()) + u128::from(fresh.get()),
        ),
    }
}

/// The AICS report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheGateReport {
    /// The contract the report is measured against.
    pub contract: MetricContract,
    /// Cache-read tokens pooled over measured samples.
    pub cached: CachedTokens,
    /// Fresh input tokens pooled over measured samples.
    pub fresh: FreshTokens,
    /// The pooled share; `None` when no sample was measured.
    pub bps: Option<BasisPoints>,
    /// The target.
    pub target: BasisPoints,
    /// Samples excluded because a required field was not reported.
    pub excluded: u64,
    /// Per-request distribution.
    pub distribution: RequestDistribution,
    /// Cache-write amplification across samples that reported writes.
    pub amplification: WriteAmplification,
}

impl CacheGateReport {
    /// The verdict against the target.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        Verdict::of(self.bps, self.target)
    }

    /// A one-line rendering that always names the contract and the exclusions.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{} {}: {} ({}) vs {} target; {} sample(s) excluded",
            self.contract.name,
            self.contract.code(),
            self.bps
                .map_or_else(|| "insufficient data".to_owned(), percent_text),
            self.contract.denominator,
            percent_text(self.target),
            self.excluded
        )
    }
}

/// Build the AICS report from samples.
#[must_use]
pub fn build_report(samples: &[CacheSample], target: BasisPoints) -> CacheGateReport {
    let mut cached = 0_u64;
    let mut fresh = 0_u64;
    let mut writes = 0_u64;
    let mut reads = 0_u64;
    let mut excluded = 0_u64;
    let mut shares = Vec::new();
    for sample in samples {
        let (Some(read), Some(fee)) = (
            sample.cached.as_reported().copied(),
            sample.fresh.as_reported().copied(),
        ) else {
            excluded += 1;
            continue;
        };
        cached = cached.saturating_add(read.get());
        fresh = fresh.saturating_add(fee.get());
        reads = reads.saturating_add(read.get());
        if let Some(write) = sample.write.as_reported().copied() {
            writes = writes.saturating_add(write.get());
        }
        if let Some(share) = BasisPoints::from_ratio(
            u128::from(read.get()),
            u128::from(read.get()) + u128::from(fee.get()),
        ) {
            shares.push(share);
        }
    }
    CacheGateReport {
        contract: aics_contract(),
        cached: CachedTokens::new(cached),
        fresh: FreshTokens::new(fresh),
        bps: BasisPoints::from_ratio(u128::from(cached), u128::from(cached) + u128::from(fresh)),
        target,
        excluded,
        distribution: RequestDistribution::of(&shares),
        amplification: write_amplification(
            CacheWriteTokens::new(writes),
            CachedTokens::new(reads),
            FreshTokens::new(fresh),
        ),
    }
}

/// One measured request for the EEPR contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixSample {
    /// Cache-read tokens served.
    pub served: Reported<CachedTokens>,
    /// Expected prompt tokens (read + fresh).
    pub expected: Reported<Tokens>,
    /// Whether the run was a cold start.
    pub cold: bool,
}

/// The EEPR report: warm-only ratio plus the cold runs reported separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixGateReport {
    /// The contract.
    pub contract: MetricContract,
    /// Cache-read tokens of warm runs.
    pub served: CachedTokens,
    /// Expected prompt tokens of warm runs.
    pub expected: Tokens,
    /// The warm ratio; `None` when no warm run was measured.
    pub bps: Option<BasisPoints>,
    /// The target.
    pub target: BasisPoints,
    /// Warm runs measured.
    pub warm_runs: u64,
    /// Cold runs excluded from the ratio.
    pub cold_runs: u64,
    /// The cold runs' own pooled share, reported but never blended into the verdict.
    pub cold_bps: Option<BasisPoints>,
    /// Samples excluded because a required field was not reported.
    pub excluded: u64,
}

impl PrefixGateReport {
    /// The verdict against the target.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        Verdict::of(self.bps, self.target)
    }

    /// A one-line rendering that names the contract and reports the cold runs separately.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{} {}: warm {} over {} warm run(s); {} cold run(s) reported separately at {}",
            self.contract.name,
            self.contract.code(),
            self.bps
                .map_or_else(|| "insufficient data".to_owned(), percent_text),
            self.warm_runs,
            self.cold_runs,
            self.cold_bps
                .map_or_else(|| "insufficient data".to_owned(), percent_text)
        )
    }
}

/// Build the EEPR report from samples.
#[must_use]
pub fn build_prefix_report(samples: &[PrefixSample], target: BasisPoints) -> PrefixGateReport {
    let mut served = 0_u64;
    let mut expected = 0_u64;
    let mut cold_served = 0_u64;
    let mut cold_expected = 0_u64;
    let mut warm_runs = 0_u64;
    let mut cold_runs = 0_u64;
    let mut excluded = 0_u64;
    for sample in samples {
        let (Some(served_tokens), Some(expected_tokens)) = (
            sample.served.as_reported().copied(),
            sample.expected.as_reported().copied(),
        ) else {
            excluded += 1;
            continue;
        };
        if sample.cold {
            cold_runs += 1;
            cold_served = cold_served.saturating_add(served_tokens.get());
            cold_expected = cold_expected.saturating_add(expected_tokens.get());
        } else {
            warm_runs += 1;
            served = served.saturating_add(served_tokens.get());
            expected = expected.saturating_add(expected_tokens.get());
        }
    }
    PrefixGateReport {
        contract: eepr_contract(),
        served: CachedTokens::new(served),
        expected: Tokens::new(expected),
        bps: BasisPoints::from_ratio(u128::from(served), u128::from(expected)),
        target,
        warm_runs,
        cold_runs,
        cold_bps: BasisPoints::from_ratio(u128::from(cold_served), u128::from(cold_expected)),
        excluded,
    }
}

/// Parse a target written as a percentage (for example `99.7`, `99.7%`, or `0.997`).
///
/// Bare values are read as percentages: `99.7` is 9 970 basis points. A value above 100 is
/// [`Error::InvalidTarget`]; a value with more than two decimal places is rounded half up.
///
/// # Errors
///
/// [`Error::InvalidTarget`] when the input is not a percentage in `0..=100`.
pub fn parse_target_percent(raw: &str) -> Result<BasisPoints, Error> {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_suffix('%').unwrap_or(trimmed).trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidTarget {
            detail: "a target percentage is required".to_owned(),
        });
    }
    let (whole, fraction) = match trimmed.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (trimmed, None),
    };
    let whole: u64 = whole.parse().map_err(|_| Error::InvalidTarget {
        detail: format!("`{raw}` is not a number"),
    })?;
    let fraction_digits = fraction.unwrap_or("");
    if fraction_digits.len() > 2 {
        return Err(Error::InvalidTarget {
            detail: format!("`{raw}` has more precision than a basis point"),
        });
    }
    let mut fraction_value: u64 = 0;
    for digit in fraction_digits.chars() {
        let digit = digit.to_digit(10).ok_or_else(|| Error::InvalidTarget {
            detail: format!("`{raw}` is not a number"),
        })?;
        fraction_value = fraction_value * 10 + u64::from(digit);
    }
    // Pad one-digit fractions to hundredths (`99.7` -> 70 hundredths).
    let hundredths = match fraction_digits.len() {
        0 => 0,
        1 => fraction_value * 10,
        _ => fraction_value,
    };
    let basis_points = whole
        .checked_mul(100)
        .and_then(|value| value.checked_add(hundredths))
        .ok_or_else(|| Error::InvalidTarget {
            detail: format!("`{raw}` overflows a basis-point value"),
        })?;
    let basis_points = u16::try_from(basis_points).map_err(|_| Error::InvalidTarget {
        detail: format!("`{raw}` is above 100%"),
    })?;
    BasisPoints::new(basis_points).ok_or_else(|| Error::InvalidTarget {
        detail: format!("`{raw}` is above 100%"),
    })
}

/// Render basis points as a percentage with two decimals (`9970` → `99.70%`).
#[must_use]
pub fn percent_text(bps: BasisPoints) -> String {
    bps.to_string()
}

/// Dollar savings from serving `cached` tokens instead of fresh input.
///
/// `cached × price_per_million_tokens_micro_usd × (1 - read_multiplier)`, in micro-USD. Prices are
/// integer micro-USD through the whole path so a savings figure is exact to the micro-dollar.
#[must_use]
pub fn dollar_savings_micro_usd(
    cached: CachedTokens,
    price_per_million_tokens_micro_usd: u64,
    read_saving: Ratio,
) -> u64 {
    let numerator = u128::from(cached.get())
        .saturating_mul(u128::from(price_per_million_tokens_micro_usd))
        .saturating_mul(read_saving.numerator());
    let denominator = 1_000_000_u128
        .saturating_mul(read_saving.denominator())
        .max(1);
    u64::try_from(numerator / denominator).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bps(value: u16) -> BasisPoints {
        BasisPoints::new(value).expect("valid basis points")
    }

    #[test]
    fn the_contracts_are_versioned_data_with_exported_exclusions() {
        let [aics, eepr] = contracts();
        assert_eq!(aics.code(), "aics.v1");
        assert_eq!(eepr.code(), "eepr.v1");
        assert!(!aics.exclusions.is_empty());
        assert!(!eepr.exclusions.is_empty());
        assert!(aics.exclusions.iter().any(|entry| entry.contains("output")));
        assert!(eepr.exclusions.iter().any(|entry| entry.contains("cold")));
    }

    #[test]
    fn aics_pools_tokens_rather_than_averaging_percentages() {
        let samples = [
            CacheSample::measured(CachedTokens::new(9_000), FreshTokens::new(1_000)),
            CacheSample::measured(CachedTokens::new(0), FreshTokens::new(1_000)),
        ];
        let report = build_report(&samples, bps(5_000));
        assert_eq!(report.bps, BasisPoints::new(8_182));
        assert_eq!(report.verdict(), Verdict::Pass);
        assert_eq!(report.distribution.count, 2);
        assert_eq!(report.distribution.min, Some(bps(0)));
        assert_eq!(report.distribution.max, Some(bps(9_000)));
        assert_eq!(report.excluded, 0);
        assert!(report.render().contains("aics.v1"));
    }

    #[test]
    fn an_unmeasured_gate_is_insufficient_not_zero_percent() {
        let report = build_report(&[], bps(5_000));
        assert_eq!(report.bps, None);
        assert_eq!(report.verdict(), Verdict::Insufficient);
        let unmeasured = build_report(
            &[CacheSample {
                cached: Reported::NotReported,
                fresh: Reported::Reported(FreshTokens::new(10)),
                write: Reported::NotReported,
            }],
            bps(5_000),
        );
        assert_eq!(unmeasured.bps, None);
        assert_eq!(unmeasured.excluded, 1);
        assert_eq!(unmeasured.verdict(), Verdict::Insufficient);
    }

    #[test]
    fn a_zero_denominator_is_insufficient_not_a_synthetic_zero() {
        let samples = [CacheSample::measured(CachedTokens::ZERO, FreshTokens::ZERO)];
        let report = build_report(&samples, bps(0));
        assert_eq!(
            report.bps, None,
            "a share of nothing is undefined, not a measured 0%"
        );
        assert_eq!(report.verdict(), Verdict::Insufficient);
    }

    #[test]
    fn eepr_excludes_cold_runs_and_reports_them_separately() {
        let samples = [
            PrefixSample {
                served: Reported::Reported(CachedTokens::new(9_000)),
                expected: Reported::Reported(Tokens::new(10_000)),
                cold: false,
            },
            PrefixSample {
                served: Reported::Reported(CachedTokens::new(0)),
                expected: Reported::Reported(Tokens::new(1_000)),
                cold: true,
            },
        ];
        let report = build_prefix_report(&samples, bps(9_500));
        assert_eq!(report.bps, Some(bps(9_000)));
        assert_eq!(report.verdict(), Verdict::Fail);
        assert_eq!(report.warm_runs, 1);
        assert_eq!(report.cold_runs, 1);
        assert_eq!(report.cold_bps, Some(bps(0)));
        assert!(report.render().contains("cold"));
    }

    #[test]
    fn the_target_parser_reads_percentages_and_refuses_nonsense() {
        assert_eq!(parse_target_percent("99.7").expect("valid"), bps(9_970));
        assert_eq!(parse_target_percent(" 99.7% ").expect("valid"), bps(9_970));
        assert_eq!(
            parse_target_percent("100").expect("valid"),
            BasisPoints::MAX
        );
        assert_eq!(parse_target_percent("0").expect("valid"), BasisPoints::ZERO);
        assert!(parse_target_percent("100.1").is_err());
        assert!(parse_target_percent("abc").is_err());
        assert!(parse_target_percent("99.999").is_err());
        assert_eq!(percent_text(bps(9_970)), "99.70%");
    }

    #[test]
    fn write_amplification_accounts_for_writes_reads_and_fresh_work() {
        let amplification = write_amplification(
            CacheWriteTokens::new(2_000),
            CachedTokens::new(1_000),
            FreshTokens::new(1_000),
        );
        assert_eq!(amplification.write_read_ratio, Ratio::new(2, 1));
        assert_eq!(amplification.churn, Ratio::new(2_000, 4_000));
        assert!(
            write_amplification(
                CacheWriteTokens::new(1),
                CachedTokens::ZERO,
                FreshTokens::ZERO
            )
            .write_read_ratio
            .is_none()
        );
    }

    #[test]
    fn dollar_savings_use_exact_integer_math() {
        // 1M cached tokens, $3.00/M fresh, read costs 0.1x -> save 0.9 * 3_000_000 = 2_700_000 µ$.
        let saving = Ratio::new(9, 10).expect("non-zero denominator");
        assert_eq!(
            dollar_savings_micro_usd(CachedTokens::new(1_000_000), 3_000_000, saving),
            2_700_000
        );
        assert_eq!(dollar_savings_micro_usd(CachedTokens::new(1), 1, saving), 0);
    }
}
