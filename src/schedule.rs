//! Pure cache-aware ordering of a batch of runs.
//!
//! Grouping work that shares a provider prompt prefix (same harness and same workspace) while its
//! cache window is warm is the cheapest scheduling improvement there is: no model call changes,
//! only the order in which already-eligible runs start. The rules are pure and deterministic —
//! `now` is the only clock, and the function performs no I/O:
//!
//! 1. The result is a permutation of `0..runs.len()`; every index appears exactly once.
//! 2. A key (harness, workspace) is warm when a [`WarmMark`] for it satisfies
//!    `0 <= now - last_finished_at <= ttl`.
//! 3. Warm keys come first, ordered by age ascending (most recently finished first), ties broken
//!    by first appearance; each warm key's members stay consecutive and in input order.
//! 4. Cold keys follow in first-appearance order, members in input order.
//! 5. Reading is deterministic: no clocks, no randomness, no I/O.

use crate::newtypes::{EpochSeconds, HarnessId, TtlSecs, WarmAgeSecs};

/// One record that a key last finished at an epoch second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmMark {
    /// The harness that finished.
    pub harness: HarnessId,
    /// The workspace (opaque caller-chosen text).
    pub workspace: String,
    /// When a run for this key finished.
    pub last_finished_at: EpochSeconds,
}

impl WarmMark {
    /// Build a mark.
    #[must_use]
    pub fn new(
        harness: HarnessId,
        workspace: impl Into<String>,
        last_finished_at: EpochSeconds,
    ) -> Self {
        Self {
            harness,
            workspace: workspace.into(),
            last_finished_at,
        }
    }

    /// The age of this mark at `now`, saturating at zero for a future instant.
    #[must_use]
    pub fn age(&self, now: EpochSeconds) -> WarmAgeSecs {
        crate::newtypes::warm_age(now, self.last_finished_at)
    }

    /// Whether the mark is inside a warm window at `now`.
    #[must_use]
    pub fn is_warm(&self, now: EpochSeconds, ttl: TtlSecs) -> bool {
        self.last_finished_at.get() <= now.get() && self.age(now).get() <= ttl.get()
    }
}

/// One item in a batch: the key its cache warmth is shared by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleKey {
    /// The harness for this item.
    pub harness: HarnessId,
    /// The workspace for this item.
    pub workspace: String,
}

impl ScheduleKey {
    /// Build a key.
    #[must_use]
    pub fn new(harness: HarnessId, workspace: impl Into<String>) -> Self {
        Self {
            harness,
            workspace: workspace.into(),
        }
    }
}

/// Compute the batch order of eligible runs, preserving prompt-cache warmth.
///
/// Returns indices into `runs`, guaranteed to be a permutation of `0..runs.len()`.
#[must_use]
pub fn schedule_order(
    runs: &[ScheduleKey],
    warm: &[WarmMark],
    now: EpochSeconds,
    ttl: TtlSecs,
) -> Vec<usize> {
    if runs.is_empty() {
        return Vec::new();
    }

    // Key order is first appearance, and each key owns its indices in input order.
    let mut key_order: Vec<(&HarnessId, &str)> = Vec::new();
    let mut key_indices: std::collections::BTreeMap<(&HarnessId, &str), Vec<usize>> =
        std::collections::BTreeMap::new();
    for (index, run) in runs.iter().enumerate() {
        let key = (&run.harness, run.workspace.as_str());
        match key_indices.get_mut(&key) {
            Some(indices) => indices.push(index),
            None => {
                key_indices.insert(key, vec![index]);
                key_order.push(key);
            }
        }
    }

    let mut warm_keys: Vec<(WarmAgeSecs, usize, (&HarnessId, &str))> = Vec::new();
    for (first_appearance, key) in key_order.iter().enumerate() {
        let qualifying = warm
            .iter()
            .filter(|mark| mark.harness == *key.0 && mark.workspace == key.1)
            .filter(|mark| mark.is_warm(now, ttl))
            .map(|mark| mark.age(now))
            .min();
        if let Some(age) = qualifying {
            warm_keys.push((age, first_appearance, *key));
        }
    }
    warm_keys.sort_by_key(|&(age, first_appearance, _)| (age, first_appearance));

    let mut order = Vec::with_capacity(runs.len());
    let mut emitted = vec![false; runs.len()];
    for (_, _, key) in warm_keys {
        if let Some(indices) = key_indices.get(&key) {
            for &index in indices {
                emitted[index] = true;
                order.push(index);
            }
        }
    }
    for (index, was_emitted) in emitted.iter().enumerate() {
        if !was_emitted {
            order.push(index);
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness(name: &str) -> HarnessId {
        HarnessId::new(name).expect("valid harness")
    }

    fn ttl() -> TtlSecs {
        TtlSecs::new(600).expect("valid ttl")
    }

    fn assert_permutation(order: &[usize], len: usize) {
        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..len).collect::<Vec<_>>());
    }

    #[test]
    fn an_empty_batch_yields_an_empty_order() {
        assert!(schedule_order(&[], &[], EpochSeconds::new(1_000), ttl()).is_empty());
    }

    #[test]
    fn a_warm_key_is_grouped_consecutively_and_comes_first() {
        let a = harness("a");
        let b = harness("b");
        let runs = vec![
            ScheduleKey::new(a.clone(), "ws1"),
            ScheduleKey::new(b.clone(), "ws2"),
            ScheduleKey::new(a.clone(), "ws1"),
        ];
        let warm = vec![WarmMark::new(a, "ws1", EpochSeconds::new(900))];
        let order = schedule_order(&runs, &warm, EpochSeconds::new(1_000), ttl());
        assert_eq!(order, vec![0, 2, 1]);
        assert_permutation(&order, runs.len());
    }

    #[test]
    fn a_stale_mark_leaves_the_key_cold() {
        let a = harness("a");
        let runs = vec![
            ScheduleKey::new(a.clone(), "ws"),
            ScheduleKey::new(a.clone(), "ws"),
        ];
        let warm = vec![WarmMark::new(a, "ws", EpochSeconds::new(100))];
        // 900 seconds old against a 600-second TTL: the step is at expiry, not a decay curve.
        assert_eq!(
            schedule_order(&runs, &warm, EpochSeconds::new(1_000), ttl()),
            vec![0, 1]
        );
    }

    #[test]
    fn warmer_keys_lead_and_ties_keep_first_appearance() {
        let a = harness("a");
        let b = harness("b");
        let c = harness("c");
        let runs = vec![
            ScheduleKey::new(a.clone(), "ws"),
            ScheduleKey::new(b.clone(), "ws"),
            ScheduleKey::new(c.clone(), "ws2"),
        ];
        let warm = vec![
            WarmMark::new(a, "ws", EpochSeconds::new(900)),
            WarmMark::new(b, "ws", EpochSeconds::new(800)),
            WarmMark::new(c, "ws2", EpochSeconds::new(900)),
        ];
        let order = schedule_order(&runs, &warm, EpochSeconds::new(1_000), ttl());
        // a (age 100) and c (age 100) tie on age; first appearance puts a first.
        assert_eq!(order, vec![0, 2, 1]);
        assert_permutation(&order, runs.len());
    }

    #[test]
    fn a_future_mark_is_not_warm() {
        let a = harness("a");
        let runs = vec![ScheduleKey::new(a.clone(), "ws")];
        let warm = vec![WarmMark::new(a, "ws", EpochSeconds::new(2_000))];
        assert_eq!(
            schedule_order(&runs, &warm, EpochSeconds::new(1_000), ttl()),
            vec![0]
        );
    }
}
