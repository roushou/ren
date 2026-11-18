//! The control API's JSON where it is a label or a shape clients read by
//! name: each run state is tagged with its phase's label, a queued prompt is
//! written as its record, and a command carries the invocation a prompt
//! writes.

use super::runs::{ClientCommand, RecordedPrompt, RunState};
use ren_harness::commands::{CommandKind, Sigil};
use ren_harness::drive::{ApprovalAsk, ApprovalId};
use ren_harness::prompt::Prompt;
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use ren_recorder::capture::RunEnd;
use ren_supervisor::{Approvals, ProcessExit, RunPhase, StopCause};
use serde_json::json;
use uuid::Uuid;

fn thread() -> NativeThreadId {
    NativeThreadId::new(Uuid::from_u128(1)).unwrap()
}

fn turn() -> NativeTurnId {
    NativeTurnId::new(Uuid::from_u128(2)).unwrap()
}

fn waiting_for(id: &str) -> Approvals {
    let mut approvals = Approvals::default();
    let id: ApprovalId = id.parse().unwrap();
    approvals.insert(id, ApprovalAsk::FileChange { reason: None });
    approvals
}

fn written(state: ren_supervisor::RunState) -> serde_json::Value {
    serde_json::to_value(RunState::from(state)).unwrap()
}

#[test]
fn a_run_state_is_tagged_with_its_phase_label() {
    for state in [
        ren_supervisor::RunState::Starting,
        ren_supervisor::RunState::Idle { thread: thread() },
        ren_supervisor::RunState::Working {
            thread: thread(),
            turn: None,
            approvals: Approvals::default(),
            queued: None,
        },
        ren_supervisor::RunState::Stopping {
            cause: StopCause::Operator,
        },
        ren_supervisor::RunState::Ended {
            end: RunEnd::Exited,
            exit: Some(ProcessExit::Code(1)),
        },
    ] {
        let phase: RunPhase = state.phase();
        assert_eq!(written(state)["phase"], phase.to_string());
    }
}

#[test]
fn a_working_run_writes_its_approvals_and_its_queued_prompt_as_its_record() {
    let working = written(ren_supervisor::RunState::Working {
        thread: thread(),
        turn: Some(turn()),
        approvals: waiting_for("ap-1"),
        queued: Some(Prompt::parse("then the docs".into()).unwrap()),
    });
    assert_eq!(working["approvals"][0]["approval"], "ap-1");
    assert_eq!(working["approvals"][0]["ask"]["kind"], "file_change");
    assert_eq!(working["queued"], "then the docs");
    let queued: RecordedPrompt = serde_json::from_value(working["queued"].clone()).unwrap();
    assert_eq!(queued, RecordedPrompt::Text("then the docs".to_owned()));
}

#[test]
fn a_command_carries_the_invocation_a_prompt_writes() {
    let command = ClientCommand::from(ren_harness::commands::ClientCommand {
        name: "review".into(),
        description: String::new(),
        hint: None,
        kind: CommandKind::Skill,
        sigil: Sigil::Dollar,
    });
    assert_eq!(
        serde_json::to_value(&command).unwrap(),
        json!({ "name": "review", "description": "", "hint": null,
                "kind": "skill", "sigil": "$", "invocation": "$review" })
    );
}
