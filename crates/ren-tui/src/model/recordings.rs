//! The gateway's sessions and model calls as last read: the newest
//! snapshot, the turns and calls read for them, and progress on calls still
//! streaming. Anything a screen shows of traffic comes from here.

use ren_harness::Harness;
use ren_protocol::activity::ActivityLimitation;
use ren_protocol::label::{Labeled, Stored};
use ren_recorder::Observation;
use ren_recorder::batch::ObservationKind;
use ren_recorder::capture::CallState;
use ren_recorder::id::{CallId, RecordingId, TurnId};
use ren_recorder::progress::Progress;
use ren_recorder::time::UnixMillis;
use ren_store::activity::{CallActivity, TurnActivity};
use ren_store::query::{CallFilter, CallQuery, KindFilter, PageLimit, StatusFilter, TimeRange};
use ren_store::view::{CallPage, CallView, RecordingView};
use std::collections::HashMap;
use std::time::Duration;

use super::Read;

/// One read of the gateway's sessions and calls.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// When the reads were made.
    pub read_at: UnixMillis,
    pub recordings: Vec<RecordingView>,
    /// Model calls still open.
    pub open: CallPage,
    /// Model calls started in the window, newest first.
    pub recent: CallPage,
}

/// The reads that make a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotQuery {
    pub recordings: PageLimit,
    pub open: CallQuery,
    pub recent: CallQuery,
}

/// One client's output over time, oldest bin first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spark {
    pub harness: Harness,
    pub bins: Vec<u64>,
}

/// The traffic, in a few numbers, for the top bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Traffic {
    /// Recordings active in the window.
    pub recordings: usize,
    /// Calls with a response streaming, and calls waiting for its head.
    pub responding: usize,
    pub awaiting: usize,
    /// Output tokens of finished calls, per client, over the last few
    /// minutes. Measured when each call ends: not a live rate.
    pub output: Vec<Spark>,
    /// Why the last read failed: what is shown is from an earlier one.
    pub failure: Option<String>,
}

/// Recordings and calls as read.
#[derive(Debug, Default)]
pub(crate) struct Recordings {
    snapshot: Option<Held>,
    turns: HashMap<TurnId, HeldTurn>,
    /// Call activity for sessions whose client sends no turns.
    activities: HashMap<CallId, CallActivity>,
    progress: HashMap<CallId, Progress>,
    /// Why the last read failed, until a snapshot succeeds.
    failure: Option<String>,
    /// Bumped by each snapshot: a screen arranges itself again when it moves.
    revision: u64,
}

#[derive(Debug)]
struct Held {
    snapshot: Snapshot,
    /// Open and recent calls, each once, oldest first.
    calls: Vec<CallView>,
}

#[derive(Debug)]
struct HeldTurn {
    /// The turn's calls and their states when it was read: it changes only
    /// when one of them does.
    signature: Vec<(CallId, Stored<CallState>)>,
    activity: TurnActivity,
}

impl Recordings {
    /// Recordings seen this recently are active.
    pub const WINDOW: Duration = Duration::from_secs(5 * 60);
    /// How far back the status line's output reaches, and in how many bins.
    pub const OUTPUT_SPAN: Duration = Duration::from_secs(4 * 60);
    pub const OUTPUT_BINS: usize = 24;

    /// The reads for a snapshot at `now`: open model calls, and those
    /// started in the window.
    pub fn query(now: UnixMillis) -> SnapshotQuery {
        let model = CallFilter {
            kind: Some(KindFilter::Model),
            ..CallFilter::default()
        };
        let from =
            UnixMillis::from_millis(now.as_millis().saturating_sub(Self::millis(Self::WINDOW)));
        SnapshotQuery {
            recordings: PageLimit::DEFAULT,
            open: CallQuery {
                filter: CallFilter {
                    status: Some(StatusFilter::Active),
                    ..model.clone()
                },
                limit: PageLimit::DEFAULT,
                after: None,
            },
            recent: CallQuery {
                filter: CallFilter {
                    // A range from a time with no end is always valid.
                    time: TimeRange::new(Some(from), None).unwrap_or_default(),
                    ..model
                },
                limit: PageLimit::LARGEST,
                after: None,
            },
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// When the snapshot was read.
    pub fn read_at(&self) -> Option<UnixMillis> {
        self.snapshot.as_ref().map(|held| held.snapshot.read_at)
    }

    pub fn recordings(&self) -> &[RecordingView] {
        self.snapshot
            .as_ref()
            .map_or(&[], |held| held.snapshot.recordings.as_slice())
    }

    /// Open and recent calls, oldest first.
    pub fn calls(&self) -> &[CallView] {
        self.snapshot.as_ref().map_or(&[], |held| &held.calls)
    }

    /// A session's calls, oldest first.
    pub fn recording_calls(&self, session: RecordingId) -> Vec<&CallView> {
        self.calls()
            .iter()
            .filter(|call| call.session == Some(session))
            .collect()
    }

    pub fn turn(&self, turn: TurnId) -> Option<&TurnActivity> {
        self.turns.get(&turn).map(|held| &held.activity)
    }

    pub fn activity(&self, call: CallId) -> Option<&CallActivity> {
        self.activities.get(&call)
    }

    pub fn progress(&self) -> &HashMap<CallId, Progress> {
        &self.progress
    }

    /// Recordings active in the window, or with a call open, or that a live
    /// run drives (by its thread): open ones first, then the most recently
    /// seen.
    pub fn active(&self, threads: &[String]) -> Vec<&RecordingView> {
        let Some(now) = self.read_at() else {
            return Vec::new();
        };
        let calls = self.calls();
        let open = |session: &RecordingView| {
            calls.iter().any(|call| {
                call.session == Some(session.id) && call.state.known() == Some(CallState::Open)
            })
        };
        let window = Self::millis(Self::WINDOW);
        let mut active: Vec<(&RecordingView, bool)> = self
            .recordings()
            .iter()
            .map(|session| (session, open(session)))
            .filter(|(session, open)| {
                *open
                    || threads.contains(&session.native_id)
                    || now
                        .as_millis()
                        .saturating_sub(session.last_seen.as_millis())
                        <= window
            })
            .collect();
        active.sort_by(|(a, a_open), (b, b_open)| {
            b_open
                .cmp(a_open)
                .then(b.last_seen.cmp(&a.last_seen))
                .then(a.id.cmp(&b.id))
        });
        active.into_iter().map(|(session, _)| session).collect()
    }

    /// The traffic at `now`.
    pub fn traffic(&self, now: UnixMillis) -> Traffic {
        let open = self
            .calls()
            .iter()
            .filter(|call| call.state.known() == Some(CallState::Open));
        let responding = open.clone().filter(|call| call.status.is_some()).count();
        Traffic {
            recordings: self.active(&[]).len(),
            responding,
            awaiting: open.count() - responding,
            output: self.output(now),
            failure: self.failure.clone(),
        }
    }

    /// What to read to show `session`: its newest two turns where they
    /// changed since they were read or are still being derived, or, for a
    /// client that sends no turns, its newest finished calls.
    pub fn reads_for(&self, session: RecordingId) -> Vec<Read> {
        /// How many of a turnless session's newest calls are read.
        const TURNLESS_CALLS: usize = 4;
        let calls = self.recording_calls(session);
        let turns = Self::recent_turns(&calls);
        let mut reads = Vec::new();
        if turns.is_empty() {
            for call in calls
                .iter()
                .rev()
                .filter(|call| call.state.known() != Some(CallState::Open))
                .take(TURNLESS_CALLS)
            {
                if !self.call_settled(call.id) {
                    reads.push(Read::Call(call.id));
                }
            }
        }
        for turn in turns {
            if !self.settled(turn) {
                reads.push(Read::Turn(turn));
            }
        }
        reads
    }

    /// The newest two turns the calls carried, oldest first.
    pub fn recent_turns(calls: &[&CallView]) -> Vec<TurnId> {
        let mut turns: Vec<TurnId> = Vec::new();
        for turn in calls.iter().rev().filter_map(|call| call.turn) {
            if !turns.contains(&turn) {
                turns.push(turn);
            }
            if turns.len() == 2 {
                break;
            }
        }
        turns.reverse();
        turns
    }

    /// Whether `turn` was read since its calls last changed, and its text
    /// is derived.
    pub fn settled(&self, turn: TurnId) -> bool {
        self.turns.get(&turn).is_some_and(|held| {
            held.signature == self.signature(turn)
                && held
                    .activity
                    .calls
                    .iter()
                    .all(|call| !Self::underived(&call.limitations))
        })
    }

    /// Whether `call`'s activity was read with its text derived.
    pub fn call_settled(&self, call: CallId) -> bool {
        self.activities
            .get(&call)
            .is_some_and(|activity| !Self::underived(&activity.limitations))
    }

    fn underived(limitations: &[Stored<ActivityLimitation>]) -> bool {
        limitations
            .iter()
            .any(|limitation| limitation.known() == Some(ActivityLimitation::NotYetDerived))
    }

    pub(super) fn take(&mut self, snapshot: Snapshot) {
        let mut calls: Vec<CallView> = Vec::new();
        for call in snapshot.open.calls.iter().chain(&snapshot.recent.calls) {
            if !calls.iter().any(|held| held.id == call.id) {
                calls.push(call.clone());
            }
        }
        calls.sort_by_key(|call| call.started_at);
        // Progress is kept only for calls that may still be streaming.
        self.progress.retain(|id, _| {
            calls
                .iter()
                .find(|call| call.id == *id)
                .is_none_or(|call| call.state.known() == Some(CallState::Open))
        });
        // Turns and calls no longer in the window are let go.
        self.turns
            .retain(|turn, _| calls.iter().any(|call| call.turn == Some(*turn)));
        self.activities
            .retain(|id, _| calls.iter().any(|call| call.id == *id));
        self.snapshot = Some(Held { snapshot, calls });
        self.failure = None;
        self.revision += 1;
    }

    pub(super) fn take_turn(&mut self, activity: TurnActivity) {
        let signature = self.signature(activity.turn);
        self.turns.insert(
            activity.turn,
            HeldTurn {
                signature,
                activity,
            },
        );
    }

    pub(super) fn take_call(&mut self, activity: CallActivity) {
        self.activities.insert(activity.call, activity);
    }

    pub(super) fn take_progress(&mut self, progress: Progress) {
        // A late update on a call already read as ended is dropped.
        let ended = self
            .calls()
            .iter()
            .find(|call| call.id == progress.call)
            .is_some_and(|call| call.state.known() != Some(CallState::Open));
        if !ended {
            self.progress.insert(progress.call, progress);
        }
    }

    pub(super) fn failed(&mut self, why: String) {
        self.failure = Some(why);
    }

    /// What an observation from the live feed calls for.
    pub(super) fn observe(&mut self, observation: &Observation) -> Vec<Read> {
        match observation.kind.known() {
            Some(ObservationKind::CallFinished | ObservationKind::CallRecovered) => {
                if let Some(call) = observation.call {
                    self.progress.remove(&call);
                }
                vec![Read::Snapshot]
            }
            Some(ObservationKind::CallOpened | ObservationKind::ResponseHead) => {
                vec![Read::Snapshot]
            }
            Some(
                ObservationKind::AttemptStarted
                | ObservationKind::AttemptFinished
                | ObservationKind::ChannelClosed
                | ObservationKind::OperatorAction
                | ObservationKind::RunRecovered,
            )
            | None => Vec::new(),
            // A run's session appears with its first call; its state is the
            // control API's.
            Some(
                ObservationKind::RunStarted
                | ObservationKind::RunOpened
                | ObservationKind::RunConfigured
                | ObservationKind::RunEnded,
            ) => vec![Read::Runs, Read::Snapshot],
            // Archived or deleted history leaves the lists.
            Some(ObservationKind::HistoryEdited) => vec![Read::Recorded, Read::Snapshot],
        }
    }

    fn signature(&self, turn: TurnId) -> Vec<(CallId, Stored<CallState>)> {
        self.calls()
            .iter()
            .filter(|call| call.turn == Some(turn))
            .map(|call| (call.id, call.state.clone()))
            .collect()
    }

    /// Output tokens of finished calls, per client, binned by when each
    /// ended. Clients with no output in the span are left out.
    pub fn output(&self, now: UnixMillis) -> Vec<Spark> {
        let calls = self.calls();
        let span = Self::millis(Self::OUTPUT_SPAN);
        let bins = u64::try_from(Self::OUTPUT_BINS).unwrap_or(1).max(1);
        let width = (span / bins).max(1);
        let start = now.as_millis().saturating_sub(span);
        Harness::ALL
            .iter()
            .filter_map(|harness| {
                let mut spark = Spark {
                    harness: *harness,
                    bins: vec![0; Self::OUTPUT_BINS],
                };
                for call in calls {
                    let attributed = call
                        .attribution
                        .hint
                        .as_ref()
                        .and_then(|hint| hint.source.known())
                        .map(|source| source.harness());
                    let (Some(ended), Some(output)) = (
                        call.ended_at,
                        call.usage.as_ref().and_then(|usage| usage.output_tokens),
                    ) else {
                        continue;
                    };
                    if attributed != Some(*harness) || ended.as_millis() < start || ended > now {
                        continue;
                    }
                    let bin = usize::try_from((ended.as_millis() - start) / width)
                        .unwrap_or(usize::MAX)
                        .min(Self::OUTPUT_BINS - 1);
                    if let Some(total) = spark.bins.get_mut(bin) {
                        *total = total.saturating_add(output);
                    }
                }
                spark.bins.iter().any(|total| *total > 0).then_some(spark)
            })
            .collect()
    }

    pub fn millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }
}
