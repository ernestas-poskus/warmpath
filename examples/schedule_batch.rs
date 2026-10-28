//! Order a batch of runs so warm prompt-cache keys are reused first.
//!
//! The grouping rule is pure: runs that share a `(harness, workspace)` key and whose last run
//! finished inside the TTL are emitted first, most recently finished first, with each key's
//! members kept together and in input order. Cold keys follow in first-appearance order.
//!
//! Run with `cargo run --example schedule_batch`.

use warmpath::newtypes::{EpochSeconds, HarnessId, TtlSecs};
use warmpath::schedule::{ScheduleKey, WarmMark, schedule_order};

fn main() {
    let codex = HarnessId::new("codex").expect("valid harness");
    let claude = HarnessId::new("claude").expect("valid harness");
    let ttl = TtlSecs::new(600).expect("a positive TTL");
    let now = EpochSeconds::new(10_000);

    // Five already-eligible runs, in the order work arrived.
    let runs = vec![
        ScheduleKey::new(codex.clone(), "repo-a"),
        ScheduleKey::new(claude.clone(), "repo-b"),
        ScheduleKey::new(codex.clone(), "repo-c"),
        ScheduleKey::new(codex.clone(), "repo-a"),
        ScheduleKey::new(claude.clone(), "repo-b"),
    ];

    // Two keys finished recently enough to still be inside the provider cache window.
    let warm = vec![
        WarmMark::new(codex.clone(), "repo-a", EpochSeconds::new(9_940)),
        WarmMark::new(claude.clone(), "repo-b", EpochSeconds::new(9_800)),
    ];

    let order = schedule_order(&runs, &warm, now, ttl);
    assert_eq!(order.len(), runs.len(), "the order is a permutation");
    for (position, index) in order.iter().enumerate() {
        let key = &runs[*index];
        println!(
            "{position}: {} @ {} (input index {index})",
            key.harness, key.workspace
        );
    }

    // repo-a is 60 s old and repo-b is 200 s old, so repo-a leads; repo-c was never warm.
    assert_eq!(order, vec![0, 3, 1, 4, 2]);
}
