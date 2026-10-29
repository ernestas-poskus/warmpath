//! Agent topology layers: node kinds, declared prefix inheritance, and child-attribution scope.
//!
//! Provider caches are keyed on request bytes. A multi-agent harness adds a second structure on
//! top of those bytes: which agent instance issued a request, whether it started its own
//! conversation or continues one, and whether a child inherited its parent's prefix. This module
//! makes that structure a declared contract instead of an inference.
//!
//! # Layers
//!
//! - **attempt** — one dispatched run. It carries the budget and the usage triple and owns no
//!   reuse claim of its own; a continuation is a new attempt over the same session.
//! - **session** — one harness conversation. [`NodeKind`] does not describe a session; a session
//!   is continued by an attempt and does not create a new identity.
//! - **node** — one agent instance: a root, a subagent, or a fork. Only a node owns a cache
//!   identity claim and a parent edge.
//! - **cohort** — sibling roots that may share a prefix write. A [`CohortKey`] groups nodes whose
//!   identity facts are equal; it is never an identity itself.
//!
//! # Declared, never inferred
//!
//! [`capabilities`] is a small, versioned table of what a harness documents. A harness it does not
//! name gets [`TopologyCapabilities::unknown`]: no inherited prefix, no claim that a parent's usage
//! covers its children, no same-prefix hold. That is the same rule the cache table follows — a
//! route the build cannot source claims nothing — applied to the topology instead of the wire.
//!
//! # Example
//!
//! ```
//! use warmpath::newtypes::HarnessId;
//! use warmpath::topology::{capabilities, inheritance, NodeKind};
//!
//! let claude = capabilities(&HarnessId::new("claude").expect("valid harness id"));
//! assert!(claude.claims_inheritance(NodeKind::Fork));
//! assert!(!claude.claims_inheritance(NodeKind::Subagent));
//!
//! let unknown = capabilities(&HarnessId::new("custom-harness").expect("valid harness id"));
//! assert_eq!(inheritance(NodeKind::Fork, &unknown).code(), "unknown");
//! ```

use serde::{Deserialize, Serialize};

use crate::newtypes::{HarnessId, ModelId, PrefixDigest, ProviderId};

/// The topology contract version this build records.
///
/// A recorded run names the version it was resolved against, so a later table change is visible
/// instead of being silently applied to old evidence.
pub const TOPOLOGY_VERSION: u32 = 1;

/// The kind of agent instance a request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A root agent: the instance a dispatch plane starts.
    Root,
    /// A subagent started by a root: its own system prompt, tools, and conversation.
    Subagent,
    /// A fork of an existing agent: a new instance whose request head is the parent's.
    Fork,
}

impl NodeKind {
    /// The stable spelling used in records and logs.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            NodeKind::Root => "root",
            NodeKind::Subagent => "subagent",
            NodeKind::Fork => "fork",
        }
    }
}

/// Whether a node's prefix continues its parent's.
///
/// The values are deliberately not a boolean: a harness can document that children share a
/// routing key without sharing prefix bytes, and "not documented" must stay distinguishable from
/// "documented as separate".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefixInheritance {
    /// The child's request head is the parent's, so the parent's write can be read.
    InheritsParent,
    /// The child starts its own prefix (own system prompt and tool surface).
    OwnPrefix,
    /// The child shares a routing/affinity key but not the prefix bytes.
    SharedRoutingKeyOnly,
    /// The harness does not document the relationship; claim nothing.
    Unknown,
}

impl PrefixInheritance {
    /// The stable spelling used in records and logs.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            PrefixInheritance::InheritsParent => "inherits_parent",
            PrefixInheritance::OwnPrefix => "own_prefix",
            PrefixInheritance::SharedRoutingKeyOnly => "shared_routing_key_only",
            PrefixInheritance::Unknown => "unknown",
        }
    }

    /// Whether this relationship permits using the parent's prefix as the child's claim.
    #[must_use]
    pub const fn claims_parent(self) -> bool {
        matches!(self, PrefixInheritance::InheritsParent)
    }
}

/// Whether a parent's reported usage already includes its children's.
///
/// This decides attribution, not caching: parsing a parent's token counts and then adding child
/// counts would double-count whenever the harness reports an inclusive total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildUsage {
    /// The parent's reported usage covers only the parent; children are separate.
    Separate,
    /// The parent's reported usage already includes its children's tokens.
    Included,
    /// The harness does not document the relationship; never infer either way.
    Unknown,
}

impl ChildUsage {
    /// The stable spelling used in records and logs.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            ChildUsage::Separate => "separate",
            ChildUsage::Included => "included",
            ChildUsage::Unknown => "unknown",
        }
    }

    /// Whether child usage may be summed onto the parent's reported total.
    #[must_use]
    pub const fn is_additive(self) -> bool {
        matches!(self, ChildUsage::Separate)
    }
}

/// The topology facts a harness documents.
///
/// The type is configuration-shaped data, like [`crate::cache::CacheContract`], not a measured
/// quantity: it has public fields and no fallible constructor. A harness the table cannot source
/// is [`TopologyCapabilities::unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TopologyCapabilities {
    /// Whether a subagent inherits the parent prefix.
    pub subagent: PrefixInheritance,
    /// Whether a fork inherits the parent prefix.
    pub fork: PrefixInheritance,
    /// Whether the harness documents a same-prefix fan-out hold.
    pub same_prefix_hold: bool,
    /// Whether the parent's usage includes its children's.
    pub child_usage: ChildUsage,
}

impl TopologyCapabilities {
    /// The conservative contract for a harness this build cannot source.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            subagent: PrefixInheritance::Unknown,
            fork: PrefixInheritance::Unknown,
            same_prefix_hold: false,
            child_usage: ChildUsage::Unknown,
        }
    }

    /// The declared inheritance for one node kind.
    ///
    /// A root always starts its own prefix; asking this about roots is useful when a record is
    /// rendered uniformly, not as a capability lookup.
    #[must_use]
    pub const fn inheritance(&self, kind: NodeKind) -> PrefixInheritance {
        match kind {
            NodeKind::Root => PrefixInheritance::OwnPrefix,
            NodeKind::Subagent => self.subagent,
            NodeKind::Fork => self.fork,
        }
    }

    /// Whether a node of this kind may claim the parent's prefix as its own.
    #[must_use]
    pub const fn claims_inheritance(&self, kind: NodeKind) -> bool {
        self.inheritance(kind).claims_parent()
    }

    /// Whether a same-prefix fan-out hold is documented for this harness.
    #[must_use]
    pub const fn has_same_prefix_hold(&self) -> bool {
        self.same_prefix_hold
    }
}

/// The declared topology capabilities for one harness.
///
/// The table encodes the verified research in `docs/research/agent-caching/agents/`: Claude Code
/// starts subagents fresh but a fork inherits, and its reported `usage` excludes subagent tokens;
/// Codex child sessions share an affinity key but not the prefix, while a full-history fork does
/// inherit; DSH forks from an identical completed-turn seed and spawns fresh. The remaining
/// harnesses — and every gateway whose request body the build cannot inspect — are unknown.
///
/// A change to an existing entry is a [`TOPOLOGY_VERSION`] bump.
#[must_use]
pub fn capabilities(harness: &HarnessId) -> TopologyCapabilities {
    match harness.as_str().trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" => TopologyCapabilities {
            subagent: PrefixInheritance::OwnPrefix,
            fork: PrefixInheritance::InheritsParent,
            same_prefix_hold: true,
            child_usage: ChildUsage::Separate,
        },
        "codex" | "openai" => TopologyCapabilities {
            subagent: PrefixInheritance::SharedRoutingKeyOnly,
            fork: PrefixInheritance::InheritsParent,
            same_prefix_hold: false,
            child_usage: ChildUsage::Unknown,
        },
        "dsh" | "deepseek" => TopologyCapabilities {
            subagent: PrefixInheritance::OwnPrefix,
            fork: PrefixInheritance::InheritsParent,
            same_prefix_hold: false,
            child_usage: ChildUsage::Unknown,
        },
        _ => TopologyCapabilities::unknown(),
    }
}

/// The declared inheritance for one node kind under one capability set.
///
/// A thin free function over [`TopologyCapabilities::inheritance`] for callers that already hold a
/// kind and a capability value separately.
#[must_use]
pub const fn inheritance(kind: NodeKind, capabilities: &TopologyCapabilities) -> PrefixInheritance {
    capabilities.inheritance(kind)
}

/// The identity facts a cohort groups on.
///
/// Two nodes belong to one cohort only when their cache identity facts are equal. The workspace is
/// deliberately absent: it is a dispatch-plane dimension, so the caller adds it (or wraps the key)
/// rather than teaching this crate about a work tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CohortKey {
    /// The canonical static-prefix digest.
    pub prefix: PrefixDigest,
    /// The provider serving the route.
    pub provider: ProviderId,
    /// The model, when one is named.
    pub model: Option<ModelId>,
}

impl CohortKey {
    /// Build a cohort key.
    #[must_use]
    pub fn new(prefix: PrefixDigest, provider: ProviderId, model: Option<ModelId>) -> Self {
        Self {
            prefix,
            provider,
            model,
        }
    }

    /// Whether two keys describe the same writable prefix.
    ///
    /// Equality is exact: a different provider, model, or prefix byte is a different cohort. A key
    /// with an unknown model and a key naming a model do not group even when the provider matches.
    #[must_use]
    pub fn shares_write_with(&self, other: &Self) -> bool {
        self == other
    }

    /// A stable digest of the cohort's identity facts, for records that carry one string.
    ///
    /// The digest is length-delimited over prefix, provider, and model; an absent model contributes
    /// no bytes (no model identifier is empty, so an unnamed model stays distinguishable).
    #[must_use]
    pub fn id(&self) -> PrefixDigest {
        crate::prefix::digest(&[
            self.prefix.as_str().as_bytes(),
            self.provider.as_str().as_bytes(),
            self.model
                .as_ref()
                .map_or(&[][..], |model| model.as_str().as_bytes()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness(name: &str) -> HarnessId {
        HarnessId::new(name).expect("test harness id is valid")
    }

    #[test]
    fn unknown_harness_claims_nothing() {
        let capabilities = capabilities(&harness("a-harness-this-build-cannot-source"));
        assert_eq!(capabilities, TopologyCapabilities::unknown());
        assert!(!capabilities.claims_inheritance(NodeKind::Subagent));
        assert!(!capabilities.claims_inheritance(NodeKind::Fork));
        assert!(!capabilities.has_same_prefix_hold());
        assert_eq!(capabilities.child_usage, ChildUsage::Unknown);
        assert!(!capabilities.child_usage.is_additive());
    }

    #[test]
    fn claude_starts_subagents_fresh_and_forks_inherit() {
        let capabilities = capabilities(&harness("claude"));
        assert_eq!(capabilities.subagent, PrefixInheritance::OwnPrefix);
        assert_eq!(capabilities.fork, PrefixInheritance::InheritsParent);
        assert!(capabilities.claims_inheritance(NodeKind::Fork));
        assert!(!capabilities.claims_inheritance(NodeKind::Subagent));
        assert!(capabilities.has_same_prefix_hold());
        assert_eq!(capabilities.child_usage, ChildUsage::Separate);
    }

    #[test]
    fn codex_children_share_a_key_but_not_the_prefix() {
        let capabilities = capabilities(&harness("codex"));
        assert_eq!(
            capabilities.subagent,
            PrefixInheritance::SharedRoutingKeyOnly
        );
        assert!(!capabilities.claims_inheritance(NodeKind::Subagent));
        assert_eq!(capabilities.fork, PrefixInheritance::InheritsParent);
        assert_eq!(capabilities.child_usage, ChildUsage::Unknown);
    }

    #[test]
    fn dsh_forks_from_a_completed_turn_seed() {
        let capabilities = capabilities(&harness("dsh"));
        assert_eq!(capabilities.subagent, PrefixInheritance::OwnPrefix);
        assert_eq!(capabilities.fork, PrefixInheritance::InheritsParent);
        assert!(!capabilities.has_same_prefix_hold());
    }

    #[test]
    fn gates_are_unknown_by_conservative_default() {
        for name in ["agy", "antigravity", "grok", "xai", "kimi", "moonshot"] {
            assert_eq!(
                capabilities(&harness(name)),
                TopologyCapabilities::unknown(),
                "{name} must stay unknown"
            );
        }
    }

    #[test]
    fn a_root_always_owns_its_prefix() {
        let capabilities = TopologyCapabilities::unknown();
        assert_eq!(
            capabilities.inheritance(NodeKind::Root),
            PrefixInheritance::OwnPrefix
        );
        assert!(!capabilities.claims_inheritance(NodeKind::Root));
        assert_eq!(
            inheritance(NodeKind::Root, &capabilities).code(),
            "own_prefix"
        );
    }

    #[test]
    fn stable_spellings_do_not_drift() {
        assert_eq!(NodeKind::Root.code(), "root");
        assert_eq!(NodeKind::Subagent.code(), "subagent");
        assert_eq!(NodeKind::Fork.code(), "fork");
        assert_eq!(PrefixInheritance::InheritsParent.code(), "inherits_parent");
        assert_eq!(PrefixInheritance::OwnPrefix.code(), "own_prefix");
        assert_eq!(
            PrefixInheritance::SharedRoutingKeyOnly.code(),
            "shared_routing_key_only"
        );
        assert_eq!(PrefixInheritance::Unknown.code(), "unknown");
        assert_eq!(ChildUsage::Separate.code(), "separate");
        assert_eq!(ChildUsage::Included.code(), "included");
        assert_eq!(ChildUsage::Unknown.code(), "unknown");
    }

    #[test]
    fn cohort_keys_group_only_equal_identity_facts() {
        let prefix = crate::prefix::digest(&[b"static head"]);
        let other = crate::prefix::digest(&[b"other head"]);
        let provider = ProviderId::new("anthropic").expect("provider");
        let model = ModelId::new("claude-sonnet-5").ok();

        let base = CohortKey::new(prefix.clone(), provider.clone(), model.clone());
        assert!(base.shares_write_with(&base.clone()));

        let changed_prefix = CohortKey::new(other, provider.clone(), model.clone());
        assert!(!base.shares_write_with(&changed_prefix));

        let changed_provider = CohortKey::new(
            prefix.clone(),
            ProviderId::new("openai").expect("provider"),
            model.clone(),
        );
        assert!(!base.shares_write_with(&changed_provider));

        let no_model = CohortKey::new(prefix, provider, None);
        assert!(
            !base.shares_write_with(&no_model),
            "unknown model is not a match"
        );
    }

    #[test]
    fn cohort_id_is_stable_and_model_sensitive() {
        let prefix = crate::prefix::digest(&[b"static head"]);
        let provider = ProviderId::new("anthropic").expect("provider");
        let model = ModelId::new("claude-sonnet-5").ok();
        let named = CohortKey::new(prefix.clone(), provider.clone(), model.clone());
        let same = CohortKey::new(prefix.clone(), provider.clone(), model);
        assert_eq!(named.id(), same.id());
        let unnamed = CohortKey::new(prefix, provider, None);
        assert_ne!(named.id(), unnamed.id());
    }

    #[test]
    fn capability_record_round_trips() {
        let capabilities = capabilities(&harness("claude"));
        let json = serde_json::to_string(&capabilities).expect("serialize");
        let decoded: TopologyCapabilities = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(capabilities, decoded);

        let kind = NodeKind::Fork;
        assert_eq!(serde_json::to_string(&kind).expect("serialize"), "\"fork\"");
    }

    #[test]
    fn version_is_recorded() {
        assert_eq!(TOPOLOGY_VERSION, 1);
    }
}
