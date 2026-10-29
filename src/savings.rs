//! Pre-provider savings attribution: what was removed **before** a request reached its provider.
//!
//! A request path can reduce prompt work in two places, in this order:
//!
//! 1. **Ingest** — content is reduced as it enters the pipeline (for example a hook layer that
//!    compacts or normalizes what a child asked to send).
//! 2. **Request** — the outgoing request is reduced on its way to the provider (for example a proxy
//!    that strips or rewrites request content).
//!
//! Both are *pre-provider* reductions: the tokens never leave the machine. This module attributes
//! those two stages and sums them, and it deliberately says nothing about provider-side prompt
//! caching — a cache read is a billing fact, not a reduction, and it is measured by
//! [`crate::gate`]/[`crate::cache`], never added to a savings total here.
//!
//! # Reported versus measured
//!
//! A reducer may know only how much it removed ([`SavingsBasis::Reported`]) or may have counted the
//! prompt before and after its stage ([`SavingsBasis::Measured`]). The two are not interchangeable:
//!
//! - [`SavingsBasis::Measured`] requires `before >= after`, and its saving is `before - after`.
//! - [`SavingsBasis::Reported`] carries a saved count and **no** before/after; the report never
//!   invents one.
//! - [`SavingsBasis::Partial`] carries a saved count across a mixed set of measured and unmeasured
//!   runs along with their counts; it is never complete on its own.
//! - The pre-provider total (the "before" figure) is only claimed when **every** stage is measured
//!   ([`SavingsReport::total_before`]).
//! - The saved total is claimed only when every observation is usable and recording. A disabled,
//!   invalid, or partial observation makes [`SavingsReport::total_saved`] `None` and the report
//!   incomplete, rather than turning an absence into a zero.
//! - An incomplete report with partial or usable observations exposes evidence through
//!   [`SavingsReport::measured_lower_bound`], never as a total.
//!
//! # Example
//!
//! ```
//! use warmpath::newtypes::Tokens;
//! use warmpath::savings::{ReductionStage, SavingsBasis, SavingsObservation, SavingsReport};
//!
//! let report = SavingsReport::from_observations(&[
//!     SavingsObservation::new(
//!         ReductionStage::Ingest,
//!         SavingsBasis::Reported {
//!             saved: Tokens::new(1_200),
//!         },
//!     ),
//!     SavingsObservation::new(
//!         ReductionStage::Request,
//!         SavingsBasis::Measured {
//!             before: Tokens::new(5_000),
//!             after: Tokens::new(4_000),
//!         },
//!     ),
//! ])
//! .expect("one observation per stage");
//! assert_eq!(report.total_saved(), Some(Tokens::new(2_200)));
//! assert!(report.is_complete());
//! assert_eq!(report.total_before(), None, "one stage was reported, not measured");
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::newtypes::{BasisPoints, Tokens};

/// Where a pre-provider reduction happens, in request order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReductionStage {
    /// Content reduced as it enters the pipeline.
    Ingest,
    /// The outgoing request reduced on its way to the provider.
    Request,
}

impl ReductionStage {
    /// The stable spelling.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            ReductionStage::Ingest => "ingest",
            ReductionStage::Request => "request",
        }
    }

    /// Positions in request order: `Ingest` precedes `Request`.
    #[must_use]
    pub const fn order(self) -> u8 {
        match self {
            ReductionStage::Ingest => 0,
            ReductionStage::Request => 1,
        }
    }

    /// Every known stage, in request order.
    #[must_use]
    pub const fn all() -> [ReductionStage; 2] {
        [ReductionStage::Ingest, ReductionStage::Request]
    }
}

impl fmt::Display for ReductionStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code())
    }
}

/// How one stage's saving is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SavingsBasis {
    /// The stage reported a saved count directly.
    Reported {
        /// Tokens the stage removed.
        saved: Tokens,
    },
    /// The stage counted the prompt before and after its reduction.
    Measured {
        /// Prompt tokens entering the stage.
        before: Tokens,
        /// Prompt tokens leaving the stage.
        after: Tokens,
    },
    /// A stage where some runs were measured and others were unmeasured.
    Partial {
        /// Tokens the measured runs removed.
        saved: Tokens,
        /// Number of measured runs.
        measured: u64,
        /// Number of unmeasured runs.
        unmeasured: u64,
    },
}

impl SavingsBasis {
    /// Whether this basis is partial (some runs were measured and others were unmeasured).
    #[must_use]
    pub const fn is_partial(&self) -> bool {
        matches!(self, SavingsBasis::Partial { .. })
    }

    /// The saving this basis establishes, or `None` when a measured pair is inconsistent.
    ///
    /// A reported or partial count is returned as reported; a measured pair is `before - after`
    /// only when `before >= after`. The report's constructor rejects an inconsistent pair, so
    /// a basis that came from a [`SavingsReport`] is always consistent.
    #[must_use]
    pub fn saving(&self) -> Option<Tokens> {
        match self {
            SavingsBasis::Reported { saved } | SavingsBasis::Partial { saved, .. } => Some(*saved),
            SavingsBasis::Measured { before, after } => before.checked_sub(*after),
        }
    }

    /// The prompt count before the stage, only when it was measured.
    ///
    /// A reported or partial saving never invents a denominator; `None` is the honest answer.
    #[must_use]
    pub const fn before(&self) -> Option<Tokens> {
        match self {
            SavingsBasis::Reported { .. } | SavingsBasis::Partial { .. } => None,
            SavingsBasis::Measured { before, .. } => Some(*before),
        }
    }

    /// The prompt count after the stage, only when it was measured.
    #[must_use]
    pub const fn after(&self) -> Option<Tokens> {
        match self {
            SavingsBasis::Reported { .. } | SavingsBasis::Partial { .. } => None,
            SavingsBasis::Measured { after, .. } => Some(*after),
        }
    }

    /// The saving as a share of the measured prompt, when both sides were measured.
    ///
    /// `None` for a reported or partial saving (there is no denominator) and for a zero measured
    /// prompt (a share of nothing is undefined).
    #[must_use]
    pub fn saving_bps(&self) -> Option<BasisPoints> {
        match self {
            SavingsBasis::Reported { .. } | SavingsBasis::Partial { .. } => None,
            SavingsBasis::Measured { before, after } => {
                let saved = before.checked_sub(*after)?;
                BasisPoints::from_ratio(u128::from(saved.get()), u128::from(before.get()))
            }
        }
    }

    /// Reject an inconsistent measured pair or an invalid partial basis.
    fn validate(&self) -> Result<(), SavingsError> {
        match self {
            SavingsBasis::Reported { .. } => Ok(()),
            SavingsBasis::Measured { before, after } if before < after => {
                Err(SavingsError::BeforeBelowAfter {
                    before: *before,
                    after: *after,
                })
            }
            SavingsBasis::Measured { .. } => Ok(()),
            SavingsBasis::Partial {
                measured,
                unmeasured,
                ..
            } if *measured == 0 || *unmeasured == 0 => Err(SavingsError::InvalidPartial {
                measured: *measured,
                unmeasured: *unmeasured,
            }),
            SavingsBasis::Partial { .. } => Ok(()),
        }
    }
}

/// One stage's observation for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingsObservation {
    /// Which reduction stage this observation describes.
    pub stage: ReductionStage,
    /// How the saving is known.
    pub basis: SavingsBasis,
    /// Requests the stage served, when it counted them.
    pub requests: Option<u64>,
    /// Whether the stage's accounting was enabled for this run.
    pub recording: bool,
}

impl SavingsObservation {
    /// An observation of an enabled stage with no request count.
    #[must_use]
    pub const fn new(stage: ReductionStage, basis: SavingsBasis) -> Self {
        Self {
            stage,
            basis,
            requests: None,
            recording: true,
        }
    }

    /// An observation of a stage where some contributions were measured and others were not.
    #[must_use]
    pub const fn partial(
        stage: ReductionStage,
        saved: Tokens,
        measured: u64,
        unmeasured: u64,
    ) -> Self {
        Self {
            stage,
            basis: SavingsBasis::Partial {
                saved,
                measured,
                unmeasured,
            },
            requests: None,
            recording: true,
        }
    }

    /// Attach a request count.
    #[must_use]
    pub const fn with_requests(mut self, requests: u64) -> Self {
        self.requests = Some(requests);
        self
    }

    /// Mark the stage's accounting disabled (or enabled) for this run.
    #[must_use]
    pub const fn with_recording(mut self, recording: bool) -> Self {
        self.recording = recording;
        self
    }

    /// Whether this observation can contribute to a total.
    ///
    /// An observation is usable when its accounting ran, its basis establishes a saving, and it
    /// is not partial. A partial basis represents incomplete evidence rather than a complete
    /// observation, so it is not usable for a total (though it contributes to
    /// [`SavingsReport::measured_lower_bound`]). The report's constructor rejects an inconsistent
    /// measured pair or invalid partial counts.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.recording && !self.basis.is_partial() && self.basis.validate().is_ok()
    }

    /// The saving this observation establishes, when it is usable.
    ///
    /// Returns `None` when the observation is not usable (disabled, invalid, or partial).
    /// Partial stage evidence is accessed through [`SavingsReport::measured_lower_bound`],
    /// never as a stage saving.
    #[must_use]
    pub fn saving(&self) -> Option<Tokens> {
        if self.is_usable() {
            self.basis.saving()
        } else {
            None
        }
    }
}

/// Why a set of observations cannot be a report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SavingsError {
    /// The same reduction stage was observed twice; a report attributes each stage once.
    #[error("the `{stage}` stage is observed twice; a report attributes each reduction stage once")]
    DuplicateStage {
        /// The duplicated stage.
        stage: ReductionStage,
    },
    /// A measured pair counted more prompt after the stage than before it.
    #[error("a measured saving cannot have before {before} below after {after}")]
    BeforeBelowAfter {
        /// Prompt tokens entering the stage.
        before: Tokens,
        /// Prompt tokens leaving the stage.
        after: Tokens,
    },
    /// A partial basis must have non-zero counts for both measured and unmeasured contributions.
    #[error(
        "a partial saving must have non-zero measured ({measured}) and unmeasured ({unmeasured}) counts"
    )]
    InvalidPartial {
        /// Number of measured runs.
        measured: u64,
        /// Number of unmeasured runs.
        unmeasured: u64,
    },
}

impl SavingsError {
    /// A stable machine-readable code for logs and metrics.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            SavingsError::DuplicateStage { .. } => "duplicate_stage",
            SavingsError::BeforeBelowAfter { .. } => "before_below_after",
            SavingsError::InvalidPartial { .. } => "invalid_partial",
        }
    }
}

/// A run's pre-provider savings across reduction stages.
///
/// Built through [`SavingsReport::from_observations`], so the invariants hold by construction:
/// at most one observation per stage, measured pairs are consistent, and the totals are derived
/// rather than trusted from the input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "SavingsDocument", try_from = "SavingsDocument")]
pub struct SavingsReport {
    observations: Vec<SavingsObservation>,
    total_saved: Option<Tokens>,
    complete: bool,
}

/// The wire document of a report: only the observations; the totals are derived on read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavingsDocument {
    observations: Vec<SavingsObservation>,
}

impl SavingsReport {
    /// Build a report from observations.
    ///
    /// # Errors
    ///
    /// [`SavingsError::DuplicateStage`] when a stage appears twice, and
    /// [`SavingsError::BeforeBelowAfter`] when a measured pair counts more after than before. An
    /// empty set is a valid, incomplete report: it claims no saving because nothing was observed.
    pub fn from_observations(observations: &[SavingsObservation]) -> Result<Self, SavingsError> {
        let mut seen = Vec::with_capacity(observations.len());
        for observation in observations {
            observation.basis.validate()?;
            if seen.contains(&observation.stage) {
                return Err(SavingsError::DuplicateStage {
                    stage: observation.stage,
                });
            }
            seen.push(observation.stage);
        }
        let complete =
            !observations.is_empty() && observations.iter().all(SavingsObservation::is_usable);
        let total_saved = complete.then(|| {
            observations
                .iter()
                .filter_map(SavingsObservation::saving)
                .fold(Tokens::ZERO, Tokens::saturating_add)
        });
        Ok(Self {
            observations: observations.to_vec(),
            total_saved,
            complete,
        })
    }

    /// Build a report; an alias of [`SavingsReport::from_observations`].
    ///
    /// # Errors
    ///
    /// As [`SavingsReport::from_observations`].
    pub fn new(observations: &[SavingsObservation]) -> Result<Self, SavingsError> {
        Self::from_observations(observations)
    }

    /// The observations, in the order they were supplied.
    #[must_use]
    pub fn observations(&self) -> &[SavingsObservation] {
        &self.observations
    }

    /// The summed pre-provider saving, or `None` when any observation is disabled, unusable, or
    /// partial.
    ///
    /// A missing total is an absence, not a zero: callers must say "unmeasured" or report a
    /// [`SavingsReport::measured_lower_bound`] rather than report `0` for it.
    #[must_use]
    pub const fn total_saved(&self) -> Option<Tokens> {
        self.total_saved
    }

    /// Whether every observation is usable and recording, so the totals are a complete attribution.
    ///
    /// An empty report is not complete: nothing was observed, so nothing is claimed.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// The observed saving for one stage.
    ///
    /// Returns `None` when the stage was not observed, was disabled, or is partial. Partial
    /// stage savings contribute to [`SavingsReport::measured_lower_bound`] rather than
    /// exposing a stage total.
    #[must_use]
    pub fn saved_at(&self, stage: ReductionStage) -> Option<Tokens> {
        self.observations
            .iter()
            .find(|observation| observation.stage == stage)
            .and_then(SavingsObservation::saving)
    }

    /// The measured pre-provider lower bound, or `None` when no observation was usable or partial.
    ///
    /// Sums the basis saving of every usable observation and every partial observation. For a
    /// complete report, this equals [`SavingsReport::total_saved`]. For an incomplete report, this
    /// provides lower-bound evidence of what was measured, never presenting it as a total.
    #[must_use]
    pub fn measured_lower_bound(&self) -> Option<Tokens> {
        let mut any = false;
        let mut sum = Tokens::ZERO;
        for observation in &self.observations {
            if (observation.is_usable()
                || (observation.recording && observation.basis.is_partial()))
                && let Some(saving) = observation.basis.saving()
            {
                any = true;
                sum = sum.saturating_add(saving);
            }
        }
        if any { Some(sum) } else { None }
    }

    /// The prompt count **before** the reductions, only when every stage was measured.
    ///
    /// The stages are sequential, so the pre-provider figure is the earliest observed stage's
    /// `before` — summing each stage's `before` would count the prompt once per stage. A reported
    /// saving has no denominator, so a mixed report cannot claim a pre-provider total; `None` says
    /// so instead of inventing one.
    #[must_use]
    pub fn total_before(&self) -> Option<Tokens> {
        if !self.complete {
            return None;
        }
        let mut earliest: Option<(u8, Tokens)> = None;
        for observation in &self.observations {
            let before = observation.basis.before()?;
            if earliest.is_none_or(|(order, _)| observation.stage.order() < order) {
                earliest = Some((observation.stage.order(), before));
            }
        }
        earliest.map(|(_, before)| before)
    }

    /// The total saving as a share of the measured pre-provider prompt.
    ///
    /// `None` unless every stage was measured ([`SavingsReport::total_before`]) and the total before
    /// is non-zero: a share of nothing is undefined.
    #[must_use]
    pub fn saved_fraction_bps(&self) -> Option<BasisPoints> {
        let before = self.total_before()?;
        let saved = self.total_saved?;
        BasisPoints::from_ratio(u128::from(saved.get()), u128::from(before.get()))
    }
}

impl TryFrom<SavingsDocument> for SavingsReport {
    type Error = SavingsError;

    fn try_from(document: SavingsDocument) -> Result<Self, Self::Error> {
        Self::from_observations(&document.observations)
    }
}

impl From<SavingsReport> for SavingsDocument {
    fn from(report: SavingsReport) -> Self {
        Self {
            observations: report.observations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reported(stage: ReductionStage, saved: u64) -> SavingsObservation {
        SavingsObservation::new(
            stage,
            SavingsBasis::Reported {
                saved: Tokens::new(saved),
            },
        )
    }

    fn measured(stage: ReductionStage, before: u64, after: u64) -> SavingsObservation {
        SavingsObservation::new(
            stage,
            SavingsBasis::Measured {
                before: Tokens::new(before),
                after: Tokens::new(after),
            },
        )
    }

    #[test]
    fn the_stages_are_ordered_ingest_before_request() {
        assert_eq!(
            ReductionStage::all(),
            [ReductionStage::Ingest, ReductionStage::Request]
        );
        assert!(ReductionStage::Ingest < ReductionStage::Request);
        assert_eq!(ReductionStage::Ingest.order(), 0);
        assert_eq!(ReductionStage::Request.order(), 1);
        assert_eq!(ReductionStage::Ingest.to_string(), "ingest");
        assert_eq!(ReductionStage::Request.to_string(), "request");
    }

    #[test]
    fn a_duplicate_stage_is_refused() {
        assert_eq!(
            SavingsReport::from_observations(&[
                reported(ReductionStage::Ingest, 10),
                measured(ReductionStage::Ingest, 20, 10),
            ]),
            Err(SavingsError::DuplicateStage {
                stage: ReductionStage::Ingest,
            })
        );
        assert_eq!(
            SavingsError::DuplicateStage {
                stage: ReductionStage::Ingest
            }
            .code(),
            "duplicate_stage"
        );
        assert!(
            SavingsError::DuplicateStage {
                stage: ReductionStage::Ingest
            }
            .to_string()
            .contains("ingest")
        );
    }

    #[test]
    fn a_measured_pair_requires_before_to_cover_after() {
        assert_eq!(
            SavingsReport::from_observations(&[measured(ReductionStage::Request, 100, 150)]),
            Err(SavingsError::BeforeBelowAfter {
                before: Tokens::new(100),
                after: Tokens::new(150),
            })
        );
        let error = SavingsError::BeforeBelowAfter {
            before: Tokens::new(1),
            after: Tokens::new(2),
        };
        assert_eq!(error.code(), "before_below_after");
        assert_eq!(
            measured(ReductionStage::Request, 100, 50).saving(),
            Some(Tokens::new(50))
        );
        assert_eq!(
            SavingsBasis::Measured {
                before: Tokens::new(100),
                after: Tokens::new(150),
            }
            .saving(),
            None
        );
    }

    #[test]
    fn a_reported_only_report_never_invents_a_before_or_an_after() {
        let report = SavingsReport::from_observations(&[reported(ReductionStage::Ingest, 1_200)])
            .expect("valid");
        assert_eq!(report.total_saved(), Some(Tokens::new(1_200)));
        assert!(report.is_complete());
        assert_eq!(report.total_before(), None);
        assert_eq!(report.saved_fraction_bps(), None);
        let observations = report.observations();
        assert_eq!(observations[0].basis.before(), None);
        assert_eq!(observations[0].basis.after(), None);
        assert_eq!(observations[0].basis.saving_bps(), None);
    }

    #[test]
    fn a_measured_only_report_claims_the_pre_provider_total_and_its_share() {
        let report = SavingsReport::from_observations(&[
            measured(ReductionStage::Ingest, 10_000, 8_000),
            measured(ReductionStage::Request, 8_000, 7_500),
        ])
        .expect("valid");
        assert_eq!(report.total_saved(), Some(Tokens::new(2_500)));
        assert_eq!(report.total_before(), Some(Tokens::new(10_000)));
        assert_eq!(report.saved_fraction_bps(), BasisPoints::new(2_500));
        assert_eq!(
            report.saved_at(ReductionStage::Request),
            Some(Tokens::new(500))
        );
        assert_eq!(
            report.saved_at(ReductionStage::Ingest),
            Some(Tokens::new(2_000))
        );
    }

    #[test]
    fn a_mixed_report_sums_but_does_not_claim_a_pre_provider_total() {
        let report = SavingsReport::from_observations(&[
            reported(ReductionStage::Ingest, 1_200),
            measured(ReductionStage::Request, 5_000, 4_000),
        ])
        .expect("valid");
        assert_eq!(report.total_saved(), Some(Tokens::new(2_200)));
        assert!(report.is_complete());
        assert_eq!(
            report.total_before(),
            None,
            "a reported stage has no denominator"
        );
        assert_eq!(report.saved_fraction_bps(), None);
    }

    #[test]
    fn a_disabled_stage_makes_the_total_and_completeness_absent() {
        let report = SavingsReport::from_observations(&[
            reported(ReductionStage::Ingest, 1_200),
            measured(ReductionStage::Request, 5_000, 4_000).with_recording(false),
        ])
        .expect("duplicate-free");
        assert_eq!(
            report.total_saved(),
            None,
            "disabled is not a measured zero"
        );
        assert!(!report.is_complete());
        assert_eq!(report.total_before(), None);
        assert_eq!(
            report.saved_at(ReductionStage::Ingest),
            Some(Tokens::new(1_200))
        );
        assert_eq!(report.saved_at(ReductionStage::Request), None);
    }

    #[test]
    fn an_empty_report_claims_nothing() {
        let report = SavingsReport::from_observations(&[]).expect("an empty set is valid");
        assert_eq!(report.total_saved(), None);
        assert!(!report.is_complete());
        assert_eq!(report.total_before(), None);
        assert!(report.observations().is_empty());
    }

    #[test]
    fn a_zero_measured_saving_is_a_measurement_not_an_absence() {
        let report =
            SavingsReport::from_observations(&[measured(ReductionStage::Ingest, 1_000, 1_000)])
                .expect("valid");
        assert_eq!(report.total_saved(), Some(Tokens::ZERO));
        assert!(report.is_complete());
        assert_eq!(report.saved_fraction_bps(), Some(BasisPoints::ZERO));
    }

    #[test]
    fn a_report_round_trips_through_serde_and_recomputes_derived_fields() {
        let report = SavingsReport::from_observations(&[
            reported(ReductionStage::Ingest, 1_200),
            measured(ReductionStage::Request, 5_000, 4_000),
        ])
        .expect("valid");
        let encoded = serde_json::to_string(&report).expect("serializes");
        assert!(
            !encoded.contains("total_saved") && !encoded.contains("complete"),
            "derived fields are not trusted from the wire: {encoded}"
        );
        let decoded: SavingsReport = serde_json::from_str(&encoded).expect("deserializes");
        assert_eq!(decoded, report);

        let duplicate = serde_json::json!({
            "observations": [
                {"stage": "ingest", "basis": {"kind": "reported", "saved": 1}, "requests": null, "recording": true},
                {"stage": "ingest", "basis": {"kind": "reported", "saved": 2}, "requests": null, "recording": true}
            ]
        });
        assert!(
            serde_json::from_value::<SavingsReport>(duplicate).is_err(),
            "a wire document cannot bypass the duplicate-stage rule"
        );
    }

    #[test]
    fn an_observation_round_trips_through_serde() {
        let observation = measured(ReductionStage::Request, 5_000, 4_000).with_requests(3);
        let encoded = serde_json::to_string(&observation).expect("serializes");
        assert_eq!(
            serde_json::from_str::<SavingsObservation>(&encoded).expect("deserializes"),
            observation
        );
        assert!(encoded.contains("\"kind\":\"measured\""));
    }

    #[test]
    fn token_bounds_are_the_crates_newtypes() {
        let observation = measured(ReductionStage::Ingest, u64::MAX, 0);
        let report = SavingsReport::from_observations(&[observation]).expect("valid");
        assert_eq!(report.total_saved(), Some(Tokens::new(u64::MAX)));
        assert_eq!(report.total_before(), Some(Tokens::new(u64::MAX)));
        assert_eq!(
            report.saved_fraction_bps(),
            Some(BasisPoints::MAX),
            "a full saving is 100% and the newtype clamps there"
        );
    }

    #[test]
    fn a_partial_basis_never_exposes_before_after_or_share() {
        let basis = SavingsBasis::Partial {
            saved: Tokens::new(500),
            measured: 2,
            unmeasured: 1,
        };
        assert_eq!(basis.saving(), Some(Tokens::new(500)));
        assert_eq!(basis.before(), None);
        assert_eq!(basis.after(), None);
        assert_eq!(basis.saving_bps(), None);
        assert!(basis.is_partial());
        assert!(
            !SavingsBasis::Reported {
                saved: Tokens::new(500)
            }
            .is_partial()
        );
    }

    #[test]
    fn a_partial_basis_requires_non_zero_measured_and_unmeasured_counts() {
        assert_eq!(
            SavingsBasis::Partial {
                saved: Tokens::new(100),
                measured: 0,
                unmeasured: 1,
            }
            .validate(),
            Err(SavingsError::InvalidPartial {
                measured: 0,
                unmeasured: 1,
            })
        );
        assert_eq!(
            SavingsBasis::Partial {
                saved: Tokens::new(100),
                measured: 1,
                unmeasured: 0,
            }
            .validate(),
            Err(SavingsError::InvalidPartial {
                measured: 1,
                unmeasured: 0,
            })
        );
        let error = SavingsError::InvalidPartial {
            measured: 0,
            unmeasured: 2,
        };
        assert_eq!(error.code(), "invalid_partial");
        assert!(error.to_string().contains("0"));
        assert!(error.to_string().contains("2"));
    }

    #[test]
    fn a_report_with_partial_ingest_and_reported_request() {
        let report = SavingsReport::from_observations(&[
            SavingsObservation::partial(ReductionStage::Ingest, Tokens::new(1_200), 2, 1),
            reported(ReductionStage::Request, 150),
        ])
        .expect("valid observations");
        assert_eq!(report.total_saved(), None);
        assert!(!report.is_complete());
        assert_eq!(report.measured_lower_bound(), Some(Tokens::new(1_350)));
        assert_eq!(report.saved_at(ReductionStage::Ingest), None);
        assert_eq!(
            report.saved_at(ReductionStage::Request),
            Some(Tokens::new(150))
        );
        assert_eq!(report.total_before(), None);
    }

    #[test]
    fn a_report_with_only_non_recording_placeholders_has_no_lower_bound() {
        let report = SavingsReport::from_observations(&[
            SavingsObservation::new(
                ReductionStage::Ingest,
                SavingsBasis::Reported {
                    saved: Tokens::ZERO,
                },
            )
            .with_recording(false),
            SavingsObservation::new(
                ReductionStage::Request,
                SavingsBasis::Reported {
                    saved: Tokens::ZERO,
                },
            )
            .with_recording(false),
        ])
        .expect("valid placeholders");
        assert_eq!(report.measured_lower_bound(), None);
        assert_eq!(report.total_saved(), None);
        assert!(!report.is_complete());
    }

    #[test]
    fn a_complete_reports_lower_bound_equals_its_total() {
        let report = SavingsReport::from_observations(&[
            reported(ReductionStage::Ingest, 1_200),
            measured(ReductionStage::Request, 5_000, 4_000),
        ])
        .expect("valid");
        assert_eq!(report.measured_lower_bound(), report.total_saved());
        assert_eq!(report.measured_lower_bound(), Some(Tokens::new(2_200)));
    }

    #[test]
    fn a_partial_whose_measured_sum_is_zero_yields_some_zero_without_becoming_complete() {
        let report = SavingsReport::from_observations(&[SavingsObservation::partial(
            ReductionStage::Ingest,
            Tokens::ZERO,
            1,
            1,
        )])
        .expect("valid partial");
        assert_eq!(report.measured_lower_bound(), Some(Tokens::ZERO));
        assert_eq!(report.total_saved(), None);
        assert!(!report.is_complete());
    }

    #[test]
    fn a_partial_observation_and_report_round_trip_through_serde() {
        let obs = SavingsObservation::partial(ReductionStage::Ingest, Tokens::new(1_200), 2, 1);
        let obs_json = serde_json::to_string(&obs).expect("serializes");
        assert!(obs_json.contains("\"kind\":\"partial\""));
        assert!(obs_json.contains("\"measured\":2"));
        assert!(obs_json.contains("\"unmeasured\":1"));
        let obs_decoded: SavingsObservation =
            serde_json::from_str(&obs_json).expect("deserializes");
        assert_eq!(obs_decoded, obs);

        let report =
            SavingsReport::from_observations(&[obs, reported(ReductionStage::Request, 150)])
                .expect("valid report");
        let report_json = serde_json::to_string(&report).expect("serializes");
        let report_decoded: SavingsReport =
            serde_json::from_str(&report_json).expect("deserializes");
        assert_eq!(report_decoded, report);
        assert_eq!(
            report_decoded.measured_lower_bound(),
            Some(Tokens::new(1_350))
        );
        assert_eq!(report_decoded.total_saved(), None);
    }
}
