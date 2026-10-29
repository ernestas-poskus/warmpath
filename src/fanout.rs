//! Pure, deterministic planning for a fan-out warm-up.
//!
//! When several runs would start cold against the same provider prefix, admitting them all at
//! once pays the prefix write once per run. A fan-out warm-up instead admits one member, lets the
//! provider cache that write, and releases the rest. This module owns only the decision:
//!
//! - [`FanoutKey`] is the grouping view the caller can compute before a route is chosen
//!   (an expected agent, a provider route, and a workspace). An empty agent or route is unknown
//!   and is never grouped, so the planner refuses to invent a cohort.
//! - [`FanoutMark`] records that a cohort already wrote (a finished run) or started writing
//!   (a live run).
//! - [`plan_fanout_warmup`] returns which candidates to admit, which to hold, and why
//!   ([`FanoutRelease`]) — the reason is evidence, not a gate.
//!
//! The planner reads no clock and performs no I/O: `now`, the TTL, and the release window are the
//! caller's values. The start match is workspace-scoped because the dispatch plane chooses the
//! final route after the cohort decision; a cohort must not dribble one member per tick when
//! routing picks a different agent than the pre-route hint. A cohort id derived from the rendered
//! request (the dispatch plane's record) is the later verification that the grouped runs really
//! share identity facts.

/// The grouping key of a fan-out cohort.
///
/// A cold cohort is a set of sibling roots that would each write the same provider prefix.
/// Wirepuller cannot see the provider's request bytes before a run starts, so the key is the
/// pre-route approximation the engine does know: the hinted agent, the configured provider, and the
/// workspace. An empty agent or route is unknown and is never grouped — the cohort refuses to
/// invent a group. This key decides who waits for whom; the run record's cohort id
/// (the dispatch plane's recorded cohort id) is derived from the rendered prefix and route instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanoutKey<'a> {
    /// The agent the run is expected to use; empty when the route is not yet known.
    pub agent: &'a str,
    /// The configured provider (the cache-scope dimension); empty when unknown.
    pub route: &'a str,
    /// The workspace root or identifier.
    pub workspace: &'a str,
}

impl FanoutKey<'_> {
    fn known(&self) -> bool {
        !self.agent.trim().is_empty() && !self.route.trim().is_empty()
    }
}

/// A cohort that already wrote (a finished run) or started writing (a live run) its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanoutMark<'a> {
    /// The cohort the mark belongs to.
    pub key: FanoutKey<'a>,
    /// Epoch second the mark was recorded.
    pub at: i64,
}

/// Why a cohort admitted or held its members this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanoutRelease {
    /// A finished run on the cohort is still inside the provider TTL.
    FinishedInTtl,
    /// A run on the cohort started at least the hold window ago.
    StartedHoldTicks,
    /// The cohort is cold: this tick admitted its first member and holds the rest.
    FirstAdmitted,
}

impl FanoutRelease {
    /// The stable spelling used in the event log.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            FanoutRelease::FinishedInTtl => "finished_in_ttl",
            FanoutRelease::StartedHoldTicks => "started_hold_ticks",
            FanoutRelease::FirstAdmitted => "first_admitted",
        }
    }
}

/// One cohort's timing decision this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanoutDecision<'a> {
    /// The cohort the decision applies to.
    pub key: FanoutKey<'a>,
    /// Why the cohort was released or why this tick starts it.
    pub release: FanoutRelease,
}

/// What one tick should admit, what it should hold, and why.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FanoutPlan<'a> {
    /// Indices to admit, in candidate order.
    pub admit: Vec<usize>,
    /// Indices held until the cohort has a start/end mark.
    pub held: Vec<usize>,
    /// One timing decision per multi-member cohort, in first-appearance order.
    pub releases: Vec<FanoutDecision<'a>>,
}

/// Plan a fan-out warm barrier over one tick's cohort candidates.
///
/// The barrier gets N same-prefix siblings onto one cache write instead of N: for a cold cohort of
/// two or more, only the first candidate is admitted; the rest wait until the cohort is released.
/// A cohort is released by a **finished** run inside the provider TTL (`finished`), or by a run
/// that **started** at least `release_after_secs` ago in the same workspace (`started`) — the best
/// available stand-in for a first-request signal, which a headless child does not expose. The
/// start match is workspace-scoped, not agent/route-scoped, because the final route is chosen after
/// the barrier: a cohort must not dribble one member per tick when routing picks a different agent
/// than the pre-route hint. Singletons, unknown keys, and released cohorts are admitted unchanged.
/// `admit` and `held` partition `0..candidates.len()`.
///
/// The function is pure: `now` is its only clock, and no I/O happens here.
#[must_use]
pub fn plan_fanout_warmup<'a>(
    candidates: &[FanoutKey<'a>],
    finished: &[FanoutMark<'a>],
    started: &[FanoutMark<'a>],
    now: i64,
    ttl_secs: i64,
    release_after_secs: i64,
) -> FanoutPlan<'a> {
    let mut plan = FanoutPlan::default();
    if candidates.is_empty() {
        return plan;
    }
    // First-appearance order keeps the plan deterministic.
    let mut groups: Vec<(FanoutKey<'a>, Vec<usize>)> = Vec::new();
    for (index, key) in candidates.iter().enumerate() {
        if !key.known() {
            plan.admit.push(index);
            continue;
        }
        match groups.iter_mut().find(|(candidate, _)| candidate == key) {
            Some((_, members)) => members.push(index),
            None => groups.push((*key, vec![index])),
        }
    }

    let age = |at: i64| now.saturating_sub(at);
    let release = |key: &FanoutKey<'a>| {
        let warm = finished
            .iter()
            .any(|mark| mark.key == *key && age(mark.at) >= 0 && age(mark.at) <= ttl_secs.max(0));
        let started = started.iter().any(|mark| {
            mark.key.workspace == key.workspace && age(mark.at) >= release_after_secs.max(0)
        });
        if warm {
            Some(FanoutRelease::FinishedInTtl)
        } else if started {
            Some(FanoutRelease::StartedHoldTicks)
        } else {
            None
        }
    };

    for (key, members) in groups {
        let released = release(&key);
        let group_released = released.is_some();
        if members.len() > 1 {
            plan.releases.push(FanoutDecision {
                key,
                release: released.unwrap_or(FanoutRelease::FirstAdmitted),
            });
        }
        for (position, index) in members.iter().enumerate() {
            if position == 0 || members.len() == 1 || group_released {
                plan.admit.push(*index);
            } else {
                plan.held.push(*index);
            }
        }
    }
    plan.admit.sort_unstable();
    plan.held.sort_unstable();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cold_group_admits_only_its_first_candidate() {
        let key = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let candidates = [key, key, key];
        let plan = plan_fanout_warmup(&candidates, &[], &[], 1_000, 600, 60);
        assert_eq!(plan.admit, vec![0]);
        assert_eq!(plan.held, vec![1, 2]);
        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].release, FanoutRelease::FirstAdmitted);
    }

    #[test]
    fn a_warm_group_admits_every_member() {
        let key = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let candidates = [key, key];
        let marks = [FanoutMark { key, at: 990 }];
        let plan = plan_fanout_warmup(&candidates, &marks, &[], 1_000, 600, 60);
        assert_eq!(plan.admit, vec![0, 1]);
        assert!(plan.held.is_empty());
        assert_eq!(plan.releases[0].release, FanoutRelease::FinishedInTtl);
    }

    #[test]
    fn a_recent_start_holds_the_group_until_the_release_window() {
        let key = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let candidates = [key, key];
        let recent = [FanoutMark { key, at: 970 }];
        let held = plan_fanout_warmup(&candidates, &[], &recent, 1_000, 600, 60);
        assert_eq!(held.admit, vec![0]);
        assert_eq!(held.held, vec![1]);

        let released = [FanoutMark { key, at: 900 }];
        let open = plan_fanout_warmup(&candidates, &[], &released, 1_000, 600, 60);
        assert_eq!(open.admit, vec![0, 1]);
        assert!(open.held.is_empty());
        assert_eq!(open.releases[0].release, FanoutRelease::StartedHoldTicks);
    }

    #[test]
    fn singletons_and_unknown_keys_are_never_held() {
        let singleton = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let unknown = FanoutKey {
            agent: "",
            route: "codex",
            workspace: "ws1",
        };
        let candidates = [singleton, unknown, unknown];
        let plan = plan_fanout_warmup(&candidates, &[], &[], 1_000, 600, 60);
        assert_eq!(plan.admit, vec![0, 1, 2]);
        assert!(plan.held.is_empty());
        assert!(
            plan.releases.is_empty(),
            "singletons have no cohort decision"
        );
    }

    #[test]
    fn route_and_workspace_split_the_groups() {
        let codex = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let other_route = FanoutKey {
            agent: "codex",
            route: "openai",
            workspace: "ws1",
        };
        let other_ws = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws2",
        };
        let candidates = [codex, other_route, other_ws, codex];
        let plan = plan_fanout_warmup(&candidates, &[], &[], 1_000, 600, 60);
        assert_eq!(plan.admit, vec![0, 1, 2]);
        assert_eq!(plan.held, vec![3]);
    }

    #[test]
    fn a_start_in_the_same_workspace_releases_even_when_the_route_differs() {
        let key = FanoutKey {
            agent: "claude",
            route: "claude",
            workspace: "ws1",
        };
        let actual = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let candidates = [key, key];
        let started = [FanoutMark {
            key: actual,
            at: 900,
        }];
        let plan = plan_fanout_warmup(&candidates, &[], &started, 1_000, 600, 60);
        assert_eq!(
            plan.admit,
            vec![0, 1],
            "routing may pick another agent; the workspace start still releases the group"
        );
        assert!(plan.held.is_empty());
    }

    #[test]
    fn a_start_in_another_workspace_does_not_release_the_group() {
        let key = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let other = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws2",
        };
        let candidates = [key, key];
        let started = [FanoutMark {
            key: other,
            at: 900,
        }];
        let plan = plan_fanout_warmup(&candidates, &[], &started, 1_000, 600, 60);
        assert_eq!(plan.admit, vec![0]);
        assert_eq!(plan.held, vec![1]);
    }

    #[test]
    fn an_expired_mark_does_not_release_a_cold_group() {
        let key = FanoutKey {
            agent: "codex",
            route: "codex",
            workspace: "ws1",
        };
        let candidates = [key, key];
        let expired = [FanoutMark { key, at: 300 }];
        let plan = plan_fanout_warmup(&candidates, &expired, &[], 1_000, 600, 60);
        assert_eq!(plan.admit, vec![0]);
        assert_eq!(plan.held, vec![1]);
    }

    #[test]
    fn admit_and_held_partition_every_candidate() {
        let a = FanoutKey {
            agent: "a",
            route: "p",
            workspace: "w",
        };
        let b = FanoutKey {
            agent: "b",
            route: "p",
            workspace: "w",
        };
        let candidates = [a, b, a, b, a, b];
        let plan = plan_fanout_warmup(&candidates, &[], &[], 1_000, 600, 60);
        let mut all: Vec<usize> = plan.admit.iter().chain(plan.held.iter()).copied().collect();
        all.sort_unstable();
        assert_eq!(all, (0..candidates.len()).collect::<Vec<_>>());
    }
}
