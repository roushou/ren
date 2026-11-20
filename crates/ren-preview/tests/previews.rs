//! The previews' steps: which wants hold slots, what starts and stops, and
//! the factory's reports, with invented commits and pull requests.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_preview::{Action, PreviewKey, PreviewState, Previews, Slot, StaleReport, Wanted};
use ren_release::{Commit, PreviewCapacity};

fn commit(digit: char) -> Commit {
    digit.to_string().repeat(40).parse().unwrap()
}

fn pull(number: u64, head: char) -> Wanted {
    Wanted {
        key: PreviewKey::Pull(number.to_string().parse().unwrap()),
        commit: commit(head),
    }
}

fn asked(head: char) -> Wanted {
    Wanted {
        key: PreviewKey::Commit(commit(head)),
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
        key: want.key.clone(),
        commit: want.commit.clone(),
    }
}

/// `previews` after each start it asked for succeeded.
fn running(previews: Previews, actions: &[Action]) -> Previews {
    let mut previews = previews;
    for action in actions {
        if let Action::Start { slot, commit, .. } = action {
            previews.started(*slot, commit).unwrap();
        }
    }
    previews
}

#[test]
fn the_first_wants_take_the_slots_in_order() {
    let wanted = [asked('a'), pull(12, 'b'), pull(9, 'c')];
    let (previews, actions) = Previews::default().want(&wanted, capacity(2));
    assert_eq!(actions, [start(0, &wanted[0]), start(1, &wanted[1])]);
    assert!(
        previews
            .previews()
            .iter()
            .all(|preview| preview.state == PreviewState::Building)
    );
    assert_eq!(previews.of(&wanted[2].key), None);
}

#[test]
fn a_new_head_starts_again_in_its_slot_and_a_closed_pull_stops() {
    let first = [pull(12, 'a'), pull(9, 'b')];
    let (previews, actions) = Previews::default().want(&first, capacity(2));
    let previews = running(previews, &actions);

    // #12 moved on; #9 closed; nothing else is wanted.
    let next = [pull(12, 'c')];
    let (previews, actions) = previews.want(&next, capacity(2));
    assert_eq!(actions, [Action::Stop(slot(1)), start(0, &next[0])]);
    assert_eq!(previews.previews().len(), 1);

    // Unchanged wants change nothing once it runs.
    let previews = running(previews, &actions);
    let (_, actions) = previews.want(&next, capacity(2));
    assert_eq!(actions, []);
}

#[test]
fn a_freed_slot_is_taken_by_the_next_want_without_a_stop() {
    let (previews, actions) = Previews::default().want(&[pull(12, 'a')], capacity(1));
    let previews = running(previews, &actions);
    let next = [pull(9, 'b')];
    let (_, actions) = previews.want(&next, capacity(1));
    assert_eq!(actions, [start(0, &next[0])]);
}

#[test]
fn a_failure_is_kept_and_not_tried_again_for_the_same_commit() {
    let wanted = [pull(12, 'a')];
    let (mut previews, _) = Previews::default().want(&wanted, capacity(1));
    previews
        .failed(slot(0), &commit('a'), "the build failed".into())
        .unwrap();
    let (previews, actions) = previews.want(&wanted, capacity(1));
    assert_eq!(actions, []);
    assert_eq!(
        previews.of(&wanted[0].key).unwrap().state,
        PreviewState::Failed {
            detail: "the build failed".into()
        }
    );
    // A new head is tried.
    let next = [pull(12, 'b')];
    let (_, actions) = previews.want(&next, capacity(1));
    assert_eq!(actions, [start(0, &next[0])]);
}

#[test]
fn a_build_left_unfinished_is_started_again() {
    let wanted = [pull(12, 'a')];
    let (previews, _) = Previews::default().want(&wanted, capacity(1));
    let (_, actions) = previews.want(&wanted, capacity(1));
    assert_eq!(actions, [start(0, &wanted[0])]);
}

#[test]
fn a_smaller_capacity_stops_what_no_longer_fits() {
    let wanted = [pull(1, 'a'), pull(2, 'b'), pull(3, 'c')];
    let (previews, actions) = Previews::default().want(&wanted, capacity(3));
    let previews = running(previews, &actions);
    let (previews, actions) = previews.want(&wanted, capacity(1));
    assert_eq!(actions, [Action::Stop(slot(1)), Action::Stop(slot(2))]);
    assert_eq!(previews.previews().len(), 1);
}

#[test]
fn a_key_wanted_twice_counts_once() {
    let wanted = [pull(12, 'a'), pull(12, 'a'), pull(9, 'b')];
    let (_, actions) = Previews::default().want(&wanted, capacity(2));
    assert_eq!(actions, [start(0, &wanted[0]), start(1, &wanted[2])]);
}

#[test]
fn a_report_about_another_build_is_refused() {
    let (mut previews, _) = Previews::default().want(&[pull(12, 'a')], capacity(1));
    assert_eq!(
        previews.started(slot(0), &commit('b')),
        Err(StaleReport {
            slot: slot(0),
            commit: commit('b')
        })
    );
    previews.started(slot(0), &commit('a')).unwrap();
    assert!(previews.started(slot(0), &commit('a')).is_err());
}

#[test]
fn previews_keep_their_file_shape() {
    let (mut previews, _) = Previews::default().want(&[pull(12, 'a'), asked('b')], capacity(2));
    previews.started(slot(0), &commit('a')).unwrap();
    previews.failed(slot(1), &commit('b'), "no".into()).unwrap();
    let json = serde_json::to_value(&previews).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "previews": [
            { "slot": 0, "key": { "pull": "12" }, "commit": "a".repeat(40), "state": { "state": "running" } },
            { "slot": 1, "key": { "commit": "b".repeat(40) }, "commit": "b".repeat(40), "state": { "state": "failed", "detail": "no" } },
        ] })
    );
    let read: Previews = serde_json::from_value(json).unwrap();
    assert_eq!(read, previews);
}
