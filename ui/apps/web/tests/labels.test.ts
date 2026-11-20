// Every closed vocabulary the UI decodes, pinned to the daemon's own labels
// (each crate's tests/fixtures/labels.txt): a label added or renamed in Rust
// fails here until the UI's array follows.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import * as api from '../src/lib/api.ts';
import * as attachments from '../src/lib/attachments.ts';
import * as exit from '../src/lib/exit.ts';
import * as previews from '../src/lib/previews.ts';
import * as pulls from '../src/lib/pulls.ts';
import * as runs from '../src/lib/runs.ts';
import * as signIn from '../src/lib/signIn.ts';
import * as tools from '../src/lib/tools.ts';

/** `<crate> <vocabulary>` → its labels, from every crate's pinned table. */
const rust = new Map<string, string[]>();
const crates = new URL('../../../../crates/', import.meta.url);
for (const crate of readdirSync(crates)) {
  const table = new URL(`${crate}/tests/fixtures/labels.txt`, crates);
  if (!existsSync(table)) continue;
  for (const line of readFileSync(table, 'utf8').split('\n')) {
    const at = line.lastIndexOf(': ');
    if (at < 0) continue;
    rust.set(`${crate} ${line.slice(0, at)}`, line.slice(at + 2).split(' '));
  }
}

const ui: [string, readonly string[]][] = [
  ['ren-protocol activity::ActivityLimitation', api.activityLimitations],
  ['ren-recorder capture::AttemptPhase', api.attemptPhases],
  ['ren-recorder capture::AttemptTerminal', api.attemptTerminals],
  ['ren-recorder capture::BodyEnding', api.bodyEndings],
  ['ren-harness tools::CallPurpose', api.callPurposes],
  ['ren-recorder capture::CallRejection', api.callRejections],
  ['ren-recorder capture::CallState', api.callStates],
  ['ren-recorder caller::CallerKind', api.callerKinds],
  ['ren-recorder capture::ChannelEnd', api.channelEnds],
  ['ren-protocol inspect::Coverage', api.coverages],
  ['ren-gateway api::preview::DecodeIntegrity', api.decodeIntegrities],
  ['ren-harness tools::EffectKind', api.effectKinds],
  ['ren-protocol route::Endpoint', api.endpoints],
  ['ren-gateway instance::Environment', api.environments],
  ['ren-release release::CheckKind', api.checkKinds],
  ['ren-release release::Decision', api.decisions],
  ['ren-gateway api::ErrorCode', api.errorCodes],
  ['ren-gateway api::ControlRefusal', api.controlRefusals],
  ['ren-store activity::report::FindingKind', api.findingKinds],
  ['ren-protocol inspect::FormatSource', api.formatSources],
  ['ren-harness ren_harness::Harness', api.harnesses],
  ['ren-gateway api::HealthStatus', api.healthStatuses],
  ['ren-harness attribution::HintIssue', api.hintIssues],
  ['ren-harness attribution::HintSource', api.hintSources],
  ['ren-protocol inspect::InputAccounting', api.inputAccountings],
  ['ren-protocol inspect::KindFamily', api.kindFamilies],
  ['ren-protocol inspect::Limitation', api.limitations],
  ['ren-protocol websocket::Payload', api.messagePayloads],
  ['ren-protocol route::Method', api.methods],
  ['ren-protocol inspect::ModelOutcome', api.modelOutcomes],
  ['ren-recorder batch::ObservationKind', api.observationKinds],
  ['ren-gateway api::preview::PreviewDecoding', api.previewDecodings],
  ['ren-gateway api::preview::PreviewExtent', api.previewExtents],
  ['ren-protocol route::Provider', api.providers],
  ['ren-gateway api::preview::RawFormat', api.rawFormats],
  ['ren-protocol encoding::ResponseEncoding', api.responseEncodings],
  ['ren-protocol encoding::ResponseFormat', api.responseFormats],
  ['ren-protocol activity::ResultStatus', api.resultStatuses],
  ['ren-recorder digest::SearchField', api.searchFields],
  ['ren-recorder secrets::SecretKind', api.secretKinds],
  ['ren-recorder capture::TerminalCause', api.terminalCauses],
  ['ren-protocol inspect::TerminalEvent', api.terminalEvents],
  ['ren-gateway api::preview::TextEnd', api.textEnds],
  ['ren-protocol route::Transport', api.transports],
  ['ren-harness attachment::ImageFormat', attachments.IMAGE_FORMATS],
  ['ren-supervisor ImageTaking', attachments.IMAGE_TAKING],
  ['ren-harness settings::ApprovalPolicy', runs.APPROVAL_POLICIES],
  ['ren-harness commands::CommandKind', runs.COMMAND_KINDS],
  ['ren-harness settings::Effort', runs.EFFORTS],
  ['ren-harness settings::Midway', runs.MIDWAY],
  ['ren-supervisor PathKind', runs.PATH_KINDS],
  ['ren-supervisor SessionPhase', runs.SESSION_PHASES],
  ['ren-harness commands::Sigil', runs.SIGILS],
  ['ren-supervisor ClientStatus', runs.clientStatuses],
  ['ren-harness drive::Decision', runs.decisions],
  ['ren-harness drive::DriveEventKind', runs.driveEventKinds],
  ['ren-harness drive::ItemKind', runs.itemKinds],
  ['ren-harness drive::ItemStatus', runs.itemStatuses],
  ['ren-harness models::DefaultChoice', runs.modelChoices],
  ['ren-supervisor PlaceKind', runs.placeKinds],
  ['ren-recorder capture::RunEnd', runs.runEnds],
  ['ren-recorder capture::ExitKind', exit.exitKinds],
  ['ren-preview preview::Reach', previews.reaches],
  ['ren-github github::CheckState', pulls.checkStates],
  ['ren-github github::ChecksState', pulls.checksStates],
  ['ren-github github::MergeState', pulls.mergeStates],
  ['ren-github github::Readiness', pulls.readinesses],
  ['ren-github github::ReviewDecision', pulls.reviewDecisions],
  ['ren-harness sign_in::SignInMethod', signIn.SIGN_IN_METHODS],
  ['ren-depot package::Kind', tools.KINDS],
  ['ren-gateway tools::ChangeKind', tools.changeKinds],
  ['ren-gateway tools::GenerationState', tools.generationStates],
  ['ren-depot package::Origin', tools.origins],
  ['ren-gateway tools::ChangeOutcome', tools.outcomes],
  ['ren-depot source::Platform', tools.platforms],
  ['ren-depot job::Stage', tools.stages],
];

test("each vocabulary the UI decodes has exactly the daemon's labels", () => {
  for (const [vocabulary, labels] of ui) {
    const pinned = rust.get(vocabulary);
    assert.ok(pinned, `${vocabulary} is not in any labels.txt`);
    assert.deepEqual([...labels].sort(), [...pinned].sort(), vocabulary);
  }
});
