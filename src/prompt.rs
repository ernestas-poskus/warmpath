//! Normalizing a provider's prompt-token shape into fresh, cached, and expected counts.
//!
//! Providers disagree about what "input tokens" means:
//!
//! - **Inclusive** — the reported input count *includes* cache reads (OpenAI/Codex).
//! - **Exclusive** — the reported input count is already fresh; cache reads are reported
//!   separately (Anthropic, Gemini, most others).
//! - **Partitioned** — fresh, read, and write counts are all reported as separate partitions, and
//!   none of them hides another.
//!
//! [`PromptTokens`] holds the reported fields verbatim and provides the **single normalization
//! point** ([`PromptTokens::fresh`], [`PromptTokens::expected`]). Nothing downstream re-derives the
//! arithmetic, and no caller ever stores a single "total input" number that would lose the shape.

use crate::newtypes::{BasisPoints, CacheWriteTokens, CachedTokens, FreshTokens, Tokens};
use crate::profile::InputShape;
use crate::reported::Reported;

/// The reported prompt-token fields plus the shape they were reported in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptTokens {
    /// The input count exactly as the provider reported it.
    pub reported_input: Reported<Tokens>,
    /// Cache-read (cached input) tokens, when reported.
    pub cache_read: Reported<CachedTokens>,
    /// Cache-write (cache creation) tokens, when reported.
    pub cache_write: Reported<CacheWriteTokens>,
    /// What the reported input count includes.
    pub shape: InputShape,
}

impl PromptTokens {
    /// Build the reported fields.
    #[must_use]
    pub const fn new(
        reported_input: Reported<Tokens>,
        cache_read: Reported<CachedTokens>,
        cache_write: Reported<CacheWriteTokens>,
        shape: InputShape,
    ) -> Self {
        Self {
            reported_input,
            cache_read,
            cache_write,
            shape,
        }
    }

    /// Fresh (uncached) input tokens, normalized from the reported shape.
    ///
    /// - Inclusive: `reported_input - cache_read`, saturating at zero when a provider reports a
    ///   read count larger than its input (an inconsistency, not a negative count).
    /// - Exclusive and Partitioned: `reported_input` is already fresh.
    ///
    /// `NotReported` when the input count is missing. A missing cache-read field never invents
    /// zero when the shape needs it: an Inclusive input whose read count is unknown has an unknown
    /// fresh share, and a synthetic zero would report a full cache miss that was never measured.
    #[must_use]
    pub fn fresh(&self) -> Reported<FreshTokens> {
        match self.shape {
            InputShape::Inclusive => match (self.reported_input, self.cache_read) {
                (Reported::NotReported, _) => Reported::NotReported,
                (Reported::Reported(_), Reported::NotReported) => Reported::NotReported,
                (Reported::Reported(input), Reported::Reported(read)) => {
                    let read = Tokens::new(read.get());
                    let fresh = input.saturating_sub(read);
                    Reported::Reported(FreshTokens::new(fresh.get()))
                }
            },
            InputShape::Exclusive | InputShape::Partitioned => self
                .reported_input
                .map(|input| FreshTokens::new(input.get())),
        }
    }

    /// The expected prompt work: the denominator a cache share is measured against.
    ///
    /// - Inclusive: the reported input count (it already contains the reads).
    /// - Exclusive: fresh input plus cache reads, both of which must be reported: one missing half
    ///   makes the total unknown, and zero is not substituted.
    /// - Partitioned: fresh input plus reads plus writes, all reported.
    ///
    /// `NotReported` when any required component is missing. A total of zero is `Reported(0)` when
    /// every component was measured as zero — an empty prompt is a measurement, not an absence.
    #[must_use]
    pub fn expected(&self) -> Reported<Tokens> {
        match self.shape {
            InputShape::Inclusive => self.reported_input,
            InputShape::Exclusive => match (self.reported_input, self.cache_read) {
                (Reported::Reported(input), Reported::Reported(read)) => {
                    Reported::Reported(input.saturating_add(Tokens::new(read.get())))
                }
                _ => Reported::NotReported,
            },
            InputShape::Partitioned => {
                match (self.reported_input, self.cache_read, self.cache_write) {
                    (
                        Reported::Reported(input),
                        Reported::Reported(read),
                        Reported::Reported(write),
                    ) => {
                        let total = input
                            .saturating_add(Tokens::new(read.get()))
                            .saturating_add(Tokens::new(write.get()));
                        Reported::Reported(total)
                    }
                    _ => Reported::NotReported,
                }
            }
        }
    }

    /// The cache-read share of expected prompt work, when both halves were reported.
    ///
    /// Measured zero is `Some(ZERO)`; an unknown read count is `None`; an expected total of zero
    /// is `None` (a share of nothing is not 0 %, it is undefined).
    #[must_use]
    pub fn cache_hit_bps(&self) -> Option<BasisPoints> {
        let read = self.cache_read.as_reported()?.get();
        let expected = self.expected().as_reported()?.get();
        BasisPoints::from_ratio(u128::from(read), u128::from(expected))
    }
}
