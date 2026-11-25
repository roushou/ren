//! The instances' one step: which wants hold slots, what starts and stops,
//! and the factory's reports, with invented commits and pull requests.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_gateway::deployment::{
    Action, DeploymentState, Deployments, Event, Reach, Slot, StaleReport, Trigger, Wanted,
};
use ren_gateway::release::PreviewCapacity;
use ren_git::Commit;
use ren_github::Readiness;

fn commit(digit: char) -> Commit {
    digit.to_string().repeat(40).parse().unwrap()
}

fn pull(number: u64, head: char) -> Wanted {
    Wanted {
        trigger: Trigger::FollowsPull(number.to_string().parse().unwrap()),
        commit: commit(head),
    }
}

fn asked(head: char) -> Wanted {
    Wanted {
        trigger: Trigger::Chosen(commit(head)),
        commit: commit(head),
    }
}

fn capacity(count: u64) -> PreviewCapacity {
    count.try_into().unwrap()
}

fn slot(index: usize) -> Slot {
    Slot::first(index + 1).last().unwrap()
}

fn start(index: usize, want: &Wanted) -> Action {
    Action::Start {
        slot: slot(index),
        trigger: want.trigger.clone(),
        commit: want.commit.clone(),
    }
}

/// `instances` once `wanted` is wanted, with room for `count`.
fn want(instances: Deployments, wanted: &[Wanted], count: u64) -> (Deployments, Vec<Action>) {
    instances
        .step(Event::Wanted {
            wanted: wanted.to_vec(),
            capacity: capacity(count),
        })
        .unwrap()
}

fn started(
    instances: Deployments,
    index: usize,
    head: char,
    reach: Reach,
) -> Result<Deployments, StaleReport> {
    let (instances, actions) = instances.step(Event::Started {
        slot: slot(index),
        commit: commit(head),
        reach,
    })?;
    assert_eq!(actions, []);
    Ok(instances)
}

fn failed(instances: Deployments, index: usize, head: char, detail: &str) -> Deployments {
    instances
        .step(Event::Failed {
            slot: slot(index),
            commit: commit(head),
            detail: detail.into(),
        })
        .unwrap()
        .0
}

/// `instances` after each start it asked for succeeded.
fn running(instances: Deployments, actions: &[Action]) -> Deployments {
    let mut instances = instances;
    for action in actions {
        if let Action::Start { slot, commit, .. } = action {
            instances = instances
                .step(Event::Started {
                    slot: *slot,
                    commit: commit.clone(),
                    reach: Reach::Tailnet,
                })
                .unwrap()
                .0;
        }
    }
    instances
}

#[test]
fn the_first_wants_take_the_slots_in_order() {
    let wanted = [asked('a'), pull(12, 'b'), pull(9, 'c')];
    let (previews, actions) = want(Deployments::default(), &wanted, 2);
    assert_eq!(actions, [start(0, &wanted[0]), start(1, &wanted[1])]);
    assert!(
        previews
            .deployments()
            .iter()
            .all(|instance| instance.state == DeploymentState::Building)
    );
    assert_eq!(previews.of(&wanted[2].trigger), None);
}

#[test]
fn a_new_head_starts_again_in_its_slot_and_a_closed_pull_stops() {
    let first = [pull(12, 'a'), pull(9, 'b')];
    let (previews, actions) = want(Deployments::default(), &first, 2);
    let previews = running(previews, &actions);

    // #12 moved on; #9 closed; nothing else is wanted.
    let next = [pull(12, 'c')];
    let (previews, actions) = want(previews, &next, 2);
    assert_eq!(actions, [Action::Stop(slot(1)), start(0, &next[0])]);
    assert_eq!(previews.deployments().len(), 1);

    // Unchanged wants change nothing once it runs.
    let previews = running(previews, &actions);
    let (_, actions) = want(previews, &next, 2);
    assert_eq!(actions, []);
}

#[test]
fn a_freed_slot_is_taken_by_the_next_want_without_a_stop() {
    let (previews, actions) = want(Deployments::default(), &[pull(12, 'a')], 1);
    let previews = running(previews, &actions);
    let next = [pull(9, 'b')];
    let (_, actions) = want(previews, &next, 1);
    assert_eq!(actions, [start(0, &next[0])]);
}

#[test]
fn a_failure_is_kept_and_not_tried_again_for_the_same_commit() {
    let wanted = [pull(12, 'a')];
    let (previews, _) = want(Deployments::default(), &wanted, 1);
    let previews = failed(previews, 0, 'a', "the build failed");
    let (previews, actions) = want(previews, &wanted, 1);
    assert_eq!(actions, []);
    assert_eq!(
        previews.of(&wanted[0].trigger).unwrap().state,
        DeploymentState::Failed {
            detail: "the build failed".into()
        }
    );
    // A new head is tried.
    let next = [pull(12, 'b')];
    let (_, actions) = want(previews, &next, 1);
    assert_eq!(actions, [start(0, &next[0])]);
}

#[test]
fn a_build_whose_outcome_is_unknown_is_started_again() {
    let wanted = [pull(12, 'a')];
    let (previews, _) = want(Deployments::default(), &wanted, 1);
    let (_, actions) = want(previews, &wanted, 1);
    assert_eq!(actions, [start(0, &wanted[0])]);
}

#[test]
fn a_smaller_capacity_stops_what_no_longer_fits() {
    let wanted = [pull(1, 'a'), pull(2, 'b'), pull(3, 'c')];
    let (previews, actions) = want(Deployments::default(), &wanted, 3);
    let previews = running(previews, &actions);
    let (previews, actions) = want(previews, &wanted, 1);
    assert_eq!(actions, [Action::Stop(slot(1)), Action::Stop(slot(2))]);
    assert_eq!(previews.deployments().len(), 1);
}

#[test]
fn a_source_wanted_twice_counts_once() {
    let wanted = [pull(12, 'a'), pull(12, 'a'), pull(9, 'b')];
    let (_, actions) = want(Deployments::default(), &wanted, 2);
    assert_eq!(actions, [start(0, &wanted[0]), start(1, &wanted[2])]);
}

#[test]
fn a_report_about_another_build_is_refused_and_changes_nothing() {
    let (instances, _) = want(Deployments::default(), &[pull(12, 'a')], 1);
    assert_eq!(
        started(instances.clone(), 0, 'b', Reach::Loopback),
        Err(StaleReport {
            slot: slot(0),
            commit: commit('b')
        })
    );
    let instances = started(instances, 0, 'a', Reach::Loopback).unwrap();
    // Reported twice: the second is about a build no longer going on.
    assert!(started(instances.clone(), 0, 'a', Reach::Loopback).is_err());
    assert!(
        instances
            .step(Event::Failed {
                slot: slot(0),
                commit: commit('a'),
                detail: "late".into(),
            })
            .is_err()
    );
}

#[test]
fn instances_keep_the_file_shape_previews_had() {
    // Production's gateway, another build than the factory's, reads it.
    let (previews, _) = want(Deployments::default(), &[pull(12, 'a'), asked('b')], 2);
    let previews = started(previews, 0, 'a', Reach::Tailnet).unwrap();
    let previews = failed(previews, 1, 'b', "no");
    let json = serde_json::to_value(&previews).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "previews": [
            { "slot": 0, "key": { "pull": "12" }, "commit": "a".repeat(40), "state": { "state": "running", "reach": "tailnet" } },
            { "slot": 1, "key": { "commit": "b".repeat(40) }, "commit": "b".repeat(40), "state": { "state": "failed", "detail": "no" } },
        ] })
    );
    let read: Deployments = serde_json::from_value(json).unwrap();
    assert_eq!(read, previews);
}

#[test]
fn a_draft_wants_no_preview_and_a_ready_pull_wants_its_head() {
    let number = "7".parse().unwrap();
    assert_eq!(
        Wanted::pull(number, commit('a'), Readiness::Ready),
        Some(pull(7, 'a'))
    );
    assert_eq!(Wanted::pull(number, commit('a'), Readiness::Draft), None);
}
