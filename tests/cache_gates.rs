//! The two cache gates, kept domain-neutral: an append-only request sequence, and a tool surface
//! that is part of the cache identity.
//!
//! Both are pure functions over caller-supplied bytes; nothing here calls a provider or knows
//! what the prompt says.

use warmpath::cache::{CacheBreakReason, CacheIdentity, first_request_break};
use warmpath::newtypes::{HarnessId, ProviderId};
use warmpath::prefix::{
    PrefixChange, ToolEntry, ToolManifest, check_append_only, compare_prefixes, digest,
};

fn harness() -> HarnessId {
    HarnessId::new("codex").expect("static harness id")
}

fn provider() -> ProviderId {
    ProviderId::new("openai").expect("static provider id")
}

fn identity(prefix: &str, manifest: &ToolManifest) -> CacheIdentity {
    CacheIdentity::new(digest(&[prefix.as_bytes()]), harness(), provider())
        .with_tool_manifest(manifest)
}

fn manifest(entries: &[(&str, &str)]) -> ToolManifest {
    let entries: Vec<ToolEntry> = entries
        .iter()
        .map(|(name, definition)| ToolEntry::new(*name, *definition))
        .collect();
    ToolManifest::new(&entries).expect("distinct tool names")
}

#[test]
fn a_growing_request_sequence_is_append_only() {
    let steps = [
        "static head with tools\n",
        "static head with tools\nuser: fix the failing test\n",
        "static head with tools\nuser: fix the failing test\ntool: parser.rs:41\n",
        "static head with tools\nuser: fix the failing test\ntool: parser.rs:41\nassistant: rerun\n",
    ];
    assert_eq!(check_append_only(&steps), Ok(()));
}

#[test]
fn a_volatile_byte_before_the_tail_breaks_the_sequence_at_the_first_violating_step() {
    let clean = [
        "static head\nuser: task\n",
        "static head\nuser: task\ntool: ok\n",
        "static head\nuser: task\ntool: ok\nassistant: next\n",
    ];
    assert_eq!(check_append_only(&clean), Ok(()));

    let mut dirty = clean;
    dirty[2] =
        "static head\ntimestamp: 2026-10-28T09:12:00Z\nuser: task\ntool: ok\nassistant: next\n";
    let variance = check_append_only(&dirty).expect_err("an inserted byte is not an extension");
    assert_eq!(variance.step, 2, "the first pair that is not append-only");
    assert_eq!(variance.difference.first_differing_byte, 12);
    assert!(!variance.difference.is_extension());
}

#[test]
fn a_truncated_request_is_a_variance_even_though_it_shares_a_prefix() {
    let steps = [
        "static head\nuser: task\ntool: ok\n",
        "static head\nuser: task\n",
    ];
    let variance = check_append_only(&steps).expect_err("a shorter request cannot reuse the cache");
    assert_eq!(variance.step, 1);
    assert!(variance.difference.is_append_only());
    assert!(!variance.difference.is_extension());
}

#[test]
fn the_tool_surface_is_part_of_the_cache_identity() {
    let read = ("read", "read a file");
    let edit = ("edit", "edit a file");

    let forward = manifest(&[read, edit]);
    let reversed = manifest(&[edit, read]);
    let renamed = manifest(&[read, ("write", "edit a file")]);

    let prefix = "static head\n";
    assert_eq!(
        identity(prefix, &forward).digest(),
        identity(prefix, &reversed).digest(),
        "discovery order is not content"
    );
    assert_ne!(
        identity(prefix, &forward).digest(),
        identity(prefix, &renamed).digest(),
        "a renamed tool is a different request surface"
    );
    assert_ne!(
        identity(prefix, &forward).digest(),
        CacheIdentity::new(digest(&[prefix.as_bytes()]), harness(), provider()).digest(),
        "a declared surface differs from an unknown one"
    );
}

#[test]
fn a_tool_change_is_attributed_as_the_tool_surface_change() {
    let previous = manifest(&[("read", "read a file"), ("edit", "edit a file")]);
    let current = manifest(&[("read", "read a file"), ("write", "edit a file")]);
    assert_eq!(
        previous.digest(),
        manifest(&[("edit", "edit a file"), ("read", "read a file")]).digest()
    );
    assert_ne!(previous.digest(), current.digest());

    let tool_change = Some(PrefixChange::Changed);
    assert_eq!(
        first_request_break(None, tool_change, true),
        Some(CacheBreakReason::ToolSurfaceChanged)
    );
    assert_eq!(
        first_request_break(Some(PrefixChange::Changed), tool_change, false),
        Some(CacheBreakReason::ToolSurfaceChanged),
        "the tool surface is named before the prefix rewrite it causes"
    );
}

#[test]
fn a_reordered_tool_list_is_not_a_prefix_change_by_itself() {
    let left = manifest(&[("read", "read a file"), ("edit", "edit a file")]);
    let right = manifest(&[("edit", "edit a file"), ("read", "read a file")]);
    assert_eq!(left.digest(), right.digest());
    assert!(compare_prefixes("a\nb\n", "a\nb\n").is_none());
}
