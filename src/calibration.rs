//! Calibrating a warm-session estimate against observations.
//!
//! A TTL is a **step**, not a decay curve: a provider cache either still holds the prefix or it
//! does not. The calibration report therefore groups observations into age bands, sums the expected
//! saving and the realized cache reads, and refuses to draw a conclusion from fewer than
//! [`MIN_WARM_SAMPLES`] warm observations per harness. An under-sampled harness is reported as
//! insufficient rather than silently averaged into a number nobody should trust.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::newtypes::{
    BasisPoints, CachedTokens, FreshTokens, HarnessId, Ratio, Tokens, WarmAgeSecs,
};
use crate::reported::Reported;

/// The minimum warm observations before a harness's realized ratio is reported.
pub const MIN_WARM_SAMPLES: u32 = 2;

/// Age bands used to group warm-resume observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeBand {
    /// Under five minutes: the shortest provider cache lifetime.
    UnderFiveMinutes,
    /// Five to ten minutes.
    FiveToTenMinutes,
    /// Ten to thirty minutes.
    TenToThirtyMinutes,
    /// Thirty to sixty minutes.
    ThirtyToSixtyMinutes,
    /// One hour and beyond.
    OverSixtyMinutes,
}

impl AgeBand {
    /// The band an age falls into.
    #[must_use]
    pub const fn of(age: WarmAgeSecs) -> Self {
        let seconds = age.get();
        if seconds < 300 {
            AgeBand::UnderFiveMinutes
        } else if seconds < 600 {
            AgeBand::FiveToTenMinutes
        } else if seconds < 1_800 {
            AgeBand::TenToThirtyMinutes
        } else if seconds < 3_600 {
            AgeBand::ThirtyToSixtyMinutes
        } else {
            AgeBand::OverSixtyMinutes
        }
    }

    /// The band's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AgeBand::UnderFiveMinutes => "<5m",
            AgeBand::FiveToTenMinutes => "5-10m",
            AgeBand::TenToThirtyMinutes => "10-30m",
            AgeBand::ThirtyToSixtyMinutes => "30-60m",
            AgeBand::OverSixtyMinutes => ">=60m",
        }
    }

    /// Every band, in age order.
    #[must_use]
    pub const fn all() -> [AgeBand; 5] {
        [
            AgeBand::UnderFiveMinutes,
            AgeBand::FiveToTenMinutes,
            AgeBand::TenToThirtyMinutes,
            AgeBand::ThirtyToSixtyMinutes,
            AgeBand::OverSixtyMinutes,
        ]
    }
}

impl fmt::Display for AgeBand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.label())
    }
}

/// The probability a warm session is still warm at `age`, given a declared TTL.
///
/// This is a **step**: 100 % strictly before the expiry instant and 0 % at or after it. There is no
/// exponential decay in the model because providers do not document one; a TTL either covers the
/// age or it does not. Without a declared TTL no warmth is claimed, so the probability is 0.
#[must_use]
pub fn warm_probability(age: WarmAgeSecs, ttl: Option<crate::newtypes::TtlSecs>) -> BasisPoints {
    match ttl {
        Some(ttl) if age.get() < ttl.get() => BasisPoints::MAX,
        _ => BasisPoints::ZERO,
    }
}

/// One warm-resume observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmResumeObservation {
    /// The harness that ran.
    pub harness: HarnessId,
    /// The age of the session at resume time.
    pub age: WarmAgeSecs,
    /// Cache-read tokens the earlier run recorded (the reusable prefix).
    pub cached: CachedTokens,
    /// Fresh input tokens the earlier run recorded.
    pub fresh: FreshTokens,
    /// The expected saving, as the warm decision estimated it.
    pub expected_saved: Tokens,
    /// Whether the run actually resumed the session.
    pub resumed: bool,
    /// Cache-read tokens the resumed run actually realized.
    pub realized_cached: Reported<CachedTokens>,
}

/// One age band's summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandSummary {
    /// The band.
    pub band: AgeBand,
    /// Observations in the band.
    pub samples: u64,
    /// Observations that actually resumed.
    pub warm_samples: u64,
    /// Summed expected saving.
    pub expected_saved: Tokens,
    /// Summed realized cache reads.
    pub realized_cached: CachedTokens,
    /// `realized / expected` in basis points, when expected is non-zero.
    pub realized_ratio: Option<BasisPoints>,
}

/// One harness's calibration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessCalibration {
    /// The harness.
    pub harness: HarnessId,
    /// Per-band summaries.
    pub bands: Vec<BandSummary>,
    /// Warm observations across all bands.
    pub warm_samples: u32,
    /// Summed expected saving.
    pub expected_saved: Tokens,
    /// Summed realized cache reads.
    pub realized_cached: CachedTokens,
    /// `realized / expected`, when expected is non-zero **and** the sample floor is met.
    pub realized_ratio: Option<BasisPoints>,
    /// Whether the harness has too few warm observations to report a ratio.
    pub insufficient: bool,
}

/// A whole calibration report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationReport {
    /// Per-harness calibration, in first-appearance order.
    pub harnesses: Vec<HarnessCalibration>,
}

impl CalibrationReport {
    /// Harnesses whose warm observations are below the floor.
    #[must_use]
    pub fn insufficient(&self) -> Vec<&HarnessId> {
        self.harnesses
            .iter()
            .filter(|calibration| calibration.insufficient)
            .map(|calibration| &calibration.harness)
            .collect()
    }

    /// A markdown rendering; empty input renders an honest empty report.
    #[must_use]
    pub fn render_markdown(&self) -> String {
        if self.harnesses.is_empty() {
            return "no warm-resume observations\n".to_owned();
        }
        let mut out = String::from(
            "| harness | band | samples | warm | expected saved | realized | realized % |\n| --- | --- | --- | --- | --- | --- | --- |\n",
        );
        for calibration in &self.harnesses {
            for band in &calibration.bands {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} |\n",
                    calibration.harness,
                    band.band,
                    band.samples,
                    band.warm_samples,
                    band.expected_saved,
                    band.realized_cached,
                    band.realized_ratio
                        .map_or_else(|| "insufficient".to_owned(), |bps| bps.to_string())
                ));
            }
            if calibration.insufficient {
                out.push_str(&format!(
                    "| {} | total | - | {} | {} | {} | insufficient (<{} warm) |\n",
                    calibration.harness,
                    calibration.warm_samples,
                    calibration.expected_saved,
                    calibration.realized_cached,
                    MIN_WARM_SAMPLES
                ));
            }
        }
        out
    }
}

/// Summarize observations into a calibration report.
#[must_use]
pub fn summarize(observations: &[WarmResumeObservation]) -> CalibrationReport {
    let mut harness_order: Vec<&HarnessId> = Vec::new();
    for observation in observations {
        if !harness_order.contains(&&observation.harness) {
            harness_order.push(&observation.harness);
        }
    }
    let harnesses = harness_order
        .into_iter()
        .map(|harness| {
            let mut bands: Vec<BandSummary> = AgeBand::all()
                .into_iter()
                .map(|band| BandSummary {
                    band,
                    samples: 0,
                    warm_samples: 0,
                    expected_saved: Tokens::ZERO,
                    realized_cached: CachedTokens::ZERO,
                    realized_ratio: None,
                })
                .collect();
            let mut warm_total: u32 = 0;
            let mut expected_total = Tokens::ZERO;
            let mut realized_total = CachedTokens::ZERO;
            for observation in observations
                .iter()
                .filter(|observation| &observation.harness == harness)
            {
                let index = bands
                    .iter()
                    .position(|summary| summary.band == AgeBand::of(observation.age))
                    .expect("every age has a band");
                let summary = &mut bands[index];
                summary.samples += 1;
                summary.expected_saved = summary
                    .expected_saved
                    .saturating_add(observation.expected_saved);
                expected_total = expected_total.saturating_add(observation.expected_saved);
                if observation.resumed {
                    summary.warm_samples += 1;
                    warm_total = warm_total.saturating_add(1);
                    if let Some(cached) = observation.realized_cached.as_reported().copied() {
                        summary.realized_cached = summary.realized_cached.saturating_add(cached);
                        realized_total = realized_total.saturating_add(cached);
                    }
                }
            }
            for summary in &mut bands {
                summary.realized_ratio = BasisPoints::from_ratio(
                    u128::from(summary.realized_cached.get()),
                    u128::from(summary.expected_saved.get()),
                );
            }
            let insufficient = warm_total < MIN_WARM_SAMPLES;
            HarnessCalibration {
                harness: harness.clone(),
                bands,
                warm_samples: warm_total,
                expected_saved: expected_total,
                realized_cached: realized_total,
                realized_ratio: if insufficient {
                    None
                } else {
                    BasisPoints::from_ratio(
                        u128::from(realized_total.get()),
                        u128::from(expected_total.get()),
                    )
                },
                insufficient,
            }
        })
        .collect();
    CalibrationReport { harnesses }
}

/// The realized-to-expected ratio as a [`Ratio`], when both totals are non-zero.
#[must_use]
pub fn realized_ratio(realized: CachedTokens, expected: Tokens) -> Option<Ratio> {
    Ratio::new(u128::from(realized.get()), u128::from(expected.get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::newtypes::TtlSecs;

    fn harness(name: &str) -> HarnessId {
        HarnessId::new(name).expect("valid harness")
    }

    fn observation(
        name: &str,
        age: u64,
        expected: u64,
        realized: u64,
        resumed: bool,
    ) -> WarmResumeObservation {
        WarmResumeObservation {
            harness: harness(name),
            age: WarmAgeSecs::new(age),
            cached: CachedTokens::new(realized),
            fresh: FreshTokens::new(1_000),
            expected_saved: Tokens::new(expected),
            resumed,
            realized_cached: Reported::Reported(CachedTokens::new(realized)),
        }
    }

    #[test]
    fn an_age_has_exactly_one_band() {
        assert_eq!(AgeBand::of(WarmAgeSecs::new(0)), AgeBand::UnderFiveMinutes);
        assert_eq!(
            AgeBand::of(WarmAgeSecs::new(299)),
            AgeBand::UnderFiveMinutes
        );
        assert_eq!(
            AgeBand::of(WarmAgeSecs::new(300)),
            AgeBand::FiveToTenMinutes
        );
        assert_eq!(
            AgeBand::of(WarmAgeSecs::new(600)),
            AgeBand::TenToThirtyMinutes
        );
        assert_eq!(
            AgeBand::of(WarmAgeSecs::new(1_800)),
            AgeBand::ThirtyToSixtyMinutes
        );
        assert_eq!(
            AgeBand::of(WarmAgeSecs::new(3_600)),
            AgeBand::OverSixtyMinutes
        );
        assert_eq!(AgeBand::all().len(), 5);
    }

    #[test]
    fn warmth_is_a_step_at_the_ttl_not_a_decay_curve() {
        let ttl = TtlSecs::new(600).expect("non-zero");
        assert_eq!(
            warm_probability(WarmAgeSecs::new(599), Some(ttl)),
            BasisPoints::MAX
        );
        assert_eq!(
            warm_probability(WarmAgeSecs::new(600), Some(ttl)),
            BasisPoints::ZERO,
            "at expiry the session is refused, with no partial credit"
        );
        assert_eq!(
            warm_probability(WarmAgeSecs::new(0), None),
            BasisPoints::ZERO
        );
    }

    #[test]
    fn a_report_refuses_to_conclude_from_one_warm_sample() {
        let report = summarize(&[observation("codex", 100, 1_000, 800, true)]);
        assert!(report.harnesses[0].insufficient);
        assert_eq!(report.harnesses[0].realized_ratio, None);
        assert_eq!(report.insufficient().len(), 1);
        assert!(report.render_markdown().contains("insufficient"));
    }

    #[test]
    fn two_warm_samples_produce_a_realized_ratio() {
        let report = summarize(&[
            observation("codex", 100, 1_000, 800, true),
            observation("codex", 200, 1_000, 600, true),
        ]);
        let calibration = &report.harnesses[0];
        assert!(!calibration.insufficient);
        assert_eq!(calibration.warm_samples, 2);
        assert_eq!(calibration.realized_ratio, BasisPoints::new(7_000));
        assert_eq!(calibration.bands[0].warm_samples, 2);
        assert!(report.insufficient().is_empty());
        assert!(report.render_markdown().contains("70.00%"));
    }

    #[test]
    fn reports_group_by_harness_and_band_and_skip_unresumed_rows_for_warmth() {
        let report = summarize(&[
            observation("codex", 100, 1_000, 500, true),
            observation("codex", 700, 1_000, 300, true),
            observation("claude", 100, 500, 0, false),
        ]);
        assert_eq!(report.harnesses.len(), 2);
        let codex = &report.harnesses[0];
        assert_eq!(codex.harness.as_str(), "codex");
        assert_eq!(
            codex
                .bands
                .iter()
                .find(|band| band.band == AgeBand::TenToThirtyMinutes)
                .expect("band")
                .samples,
            1
        );
        let claude = &report.harnesses[1];
        assert_eq!(claude.warm_samples, 0);
        assert!(claude.insufficient);
    }

    #[test]
    fn an_empty_report_is_honest() {
        let report = summarize(&[]);
        assert!(report.harnesses.is_empty());
        assert_eq!(report.render_markdown(), "no warm-resume observations\n");
        assert_eq!(realized_ratio(CachedTokens::new(1), Tokens::ZERO), None);
    }
}
