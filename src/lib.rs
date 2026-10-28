//! `warmpath` — cache-and-reuse primitives for applications that orchestrate coding agents.
//!
//! The crate is **domain-neutral**: it knows tokens, providers, harnesses, sessions, metrics, and
//! files, and nothing about a particular product or workflow. It packages the prompt-cache
//! accounting an orchestration layer builds around an external coding agent, so the same rules can
//! be reused and audited.
//!
//! # Two hard design rules
//!
//! 1. **Every public quantity is a newtype with a private field.** A token count, a ratio, a
//!    basis-point value, an identifier, or a digest cannot be confused with another, cannot be
//!    constructed without its validation, and cannot silently overflow: checked and saturating
//!    operations are explicit methods rather than `+`.
//! 2. **Only artifact filesystem work is asynchronous.** Parsing provider output, normalizing
//!    token shapes, canonicalizing a prefix, computing metrics, and deciding warm-session reuse
//!    are pure synchronous functions. [`artifact::ArtifactStore`] uses `tokio::fs` and
//!    `tokio::io::AsyncWriteExt`; the crate never starts a runtime and never blocks a thread on
//!    `std::fs` from an async path.
//!
//! # What is measured, and what is not
//!
//! Provider cache accounting is *reported*, not derived: a harness that does not print a field
//! leaves it [`Reported::NotReported`], and a field it printed as zero is
//! [`Reported::Reported`] zero. The two are never conflated — see [`prompt::PromptTokens`] for
//! the single normalization point that turns a provider's shape into fresh, cached, and expected
//! token counts.
//!
//! # Example
//!
//! ```
//! use warmpath::profile::HarnessProfile;
//! use warmpath::usage::parse;
//!
//! let profile = HarnessProfile::codex();
//! let output = br#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":900,"output_tokens":40}}"#;
//! let outcome = parse(&profile, output);
//! let fresh = outcome.usage.prompt.fresh().as_reported().copied();
//! assert_eq!(fresh.map(|tokens| tokens.get()), Some(300));
//!
//! // Codex reports an inclusive input count: 1200 in total, 900 served from cache.
//! // A run that printed no usage leaves the field absent. Absence is not a measured zero.
//! let silent = parse(&profile, br#"{"type":"turn.completed"}"#);
//! assert!(silent.usage.prompt.cache_read.as_reported().is_none());
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod artifact;
pub mod cache;
pub mod calibration;
pub mod error;
pub mod gate;
pub mod newtypes;
pub mod prefix;
pub mod profile;
pub mod prompt;
pub mod reported;
pub mod savings;
pub mod schedule;
pub mod usage;
pub mod warm;

pub use error::Error;
pub use newtypes::{
    ArtifactDigest, BasisPoints, CacheWriteTokens, CachedTokens, ContractId, ContractVersion,
    EpochSeconds, FreshTokens, HarnessId, HarnessVersion, ModelId, OutputTokens, PrefixDigest,
    ProfileId, ProfileVersion, ProviderId, Ratio, ReasoningTokens, RevisionId, SubjectId, Tokens,
    TtlSecs, WarmAgeSecs,
};
pub use profile::{HarnessProfile, InputShape, OutputShape};
pub use prompt::PromptTokens;
pub use reported::Reported;
pub use savings::{ReductionStage, SavingsBasis, SavingsError, SavingsObservation, SavingsReport};

/// The crate version, recorded in artifact provenance so a reader can tell which build wrote it.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
