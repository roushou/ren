// Agent runs: what the daemon's control API says about them (through this
// UI server's `/control/runs` proxy), what the recorded history keeps, and
// the Runs view's pure logic: the live conversation built from run events,
// what each run can be asked, and how the sidebar groups runs by folder.
// No browser APIs beyond the `fetch` handed in; tested in tests/runs.test.ts.
import {
  API_VERSION,
  harnesses,
  RecordingId,
  toolEffect,
  versioned,
  type Harness,
  type NativeThreadId,
  type NativeTurnId,
  type Project as RegisteredProject,
  type Shelf,
  type ToolEffect,
  type UnixMillis,
} from './api.ts';
import {
  checkedSignIn,
  finishedSignIn,
  signInMethod,
  signInState,
  startedSignIn,
  waitedSignIn,
  type Login,
  type LoginOutcome,
  type SignInMethod,
  type SignInState,
  type Waited,
} from './signIn.ts';
import {
  Uuid,
  array,
  assertNever,
  boolean,
  branded,
  brandedCount,
  count,
  isRecord,
  label,
  nullable,
  object,
  string,
  unrecognised,
  ShapeError,
  type Brand,
  type Decoder,
  type Stored,
  type AnyDecoder,
  absentAs,
  tagValue,
  tagged,
  type Variant,
} from './decode.ts';
import {
  AttachmentId,
  IMAGE_TAKING,
  attachmentRead,
  keptList,
  uploaded,
  type AttachmentRead,
  type ImageTaking,
  type KeptAttachment,
} from './attachments.ts';
import type { Mark } from './marks.ts';
import { Refusals, type Refusal } from './refusal.ts';
import { processExit, type ProcessExit } from './exit.ts';
import { Questions, type Question, type QuestionKey } from './questions.ts';
import type { ApprovalAsk as WireApprovalAsk } from './wire/ApprovalAsk.ts';
import type { ReplyRequest as WireReplyRequest } from './wire/ReplyRequest.ts';
import type { RawConversationQuery as WireRawConversationQuery } from './wire/RawConversationQuery.ts';
import type { CommandIntent as WireCommandIntent } from './wire/CommandIntent.ts';
import type { CommandPart as WireCommandPart } from './wire/CommandPart.ts';
import type { ConversationView as WireConversationView } from './wire/ConversationView.ts';
import type { DriveEvent as WireDriveEvent } from './wire/DriveEvent.ts';
import type { DriveItem as WireDriveItem } from './wire/DriveItem.ts';
import type { ItemRead as WireItemRead } from './wire/ItemRead.ts';
import type { TurnEnd as WireTurnEnd } from './wire/TurnEnd.ts';
import type { RunEntryView as WireRunEntryView } from './wire/RunEntryView.ts';
import type { RunView as WireRunView } from './wire/RunView.ts';
import type { Client as WireClient } from './wire/Client.ts';
import type { ClientCommand as WireClientCommand } from './wire/ClientCommand.ts';
import type { ClientsBody as WireClientsBody } from './wire/ClientsBody.ts';
import type { CommandsBody as WireCommandsBody } from './wire/CommandsBody.ts';
import type { Created as WireCreated } from './wire/Created.ts';
import type { WorktreeBody as WireWorktree } from './wire/WorktreeBody.ts';
import type { FileMatch as WireFileMatch } from './wire/FileMatch.ts';
import type { FilesBody as WireFilesBody } from './wire/FilesBody.ts';
import type { Model as WireModel } from './wire/Model.ts';
import type { ModelsBody as WireModelsBody } from './wire/ModelsBody.ts';
import type { OneSession as WireOneSession } from './wire/OneSession.ts';
import type { Places as WirePlaces } from './wire/Places.ts';
import type { Place as WirePlace } from './wire/Place.ts';
import type { RunSettings as WireRunSettings } from './wire/RunSettings.ts';
import type { RunSnapshot as WireRunSnapshot } from './wire/RunSnapshot.ts';
import type { RunsBody as WireRunsBody } from './wire/RunsBody.ts';
import type { SessionBody as WireSessionBody } from './wire/SessionBody.ts';
import type { SessionSnapshot as WireSessionSnapshot } from './wire/SessionSnapshot.ts';
import type { SessionState as WireSessionState } from './wire/SessionState.ts';
import type { SessionView as WireSessionView } from './wire/SessionView.ts';
import type { SessionsBody as WireSessionsBody } from './wire/SessionsBody.ts';
import type { Waiting as WireWaiting } from './wire/Waiting.ts';
import type { RunState as WireRunState } from './wire/RunState.ts';
import type { PlaceGit as WirePlaceGit } from './wire/PlaceGit.ts';
import type { RunUpdate as WireRunUpdate } from './wire/RunUpdate.ts';
import type { AnswerRequest as WireAnswerRequest } from './wire/AnswerRequest.ts';
import type { ClientRequest as WireClientRequest } from './wire/ClientRequest.ts';
import type { CreateRequest as WireCreateRequest } from './wire/CreateRequest.ts';
import type { FolderRequest as WireFolderRequest } from './wire/FolderRequest.ts';
import type { LoginAnswer as WireLoginAnswer } from './wire/LoginAnswer.ts';
import type { PromptRequest as WirePromptRequest } from './wire/PromptRequest.ts';
import type { SettingsRequest as WireSettingsRequest } from './wire/SettingsRequest.ts';
import type { SignInStart as WireSignInStart } from './wire/SignInStart.ts';
import { query } from './query.ts';
import type { AttachRequest as WireAttachRequest } from './wire/AttachRequest.ts';
import type { FilesRequest as WireFilesRequest } from './wire/FilesRequest.ts';
import type { ListRequest as WireListRequest } from './wire/ListRequest.ts';
import type { RawShelfQuery as WireRawShelfQuery } from './wire/RawShelfQuery.ts';
import type { ItemPhase as WireItemPhase } from './wire/ItemPhase.ts';
import type { RecordedPrompt as WireRecordedPrompt } from './wire/RecordedPrompt.ts';
import type { Retry as WireRetry } from './wire/Retry.ts';

export type { ProcessExit };

export type RunId = Brand<string, 'RunId'>;
export const RunId = new Uuid<'RunId'>();
/** The client's own id for a request waiting on the operator. */
/** A session an operator works in: not a recording. */
export type SessionId = Brand<string, 'SessionId'>;
export const SessionId = new Uuid<'SessionId'>();
export type ApprovalId = Brand<string, 'ApprovalId'>;
/** A turn item, named as the client named it. */
export type ItemId = Brand<string, 'ItemId'>;

export const runEnds = [
  'spawn_failed',
  'open_refused',
  'exited',
  'stopped',
  'drained',
  'recovered',
  'idled',
] as const;
export type RunEnd = (typeof runEnds)[number];
const stopCauses = ['operator', 'shutdown', 'open_refused', 'idle'] as const;
export type StopCause = (typeof stopCauses)[number];
export const decisions = ['accept', 'accept_for_session', 'decline', 'cancel'] as const;
/** How much a model reasons, from none to the most. */
export const EFFORTS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as const;
export type Effort = (typeof EFFORTS)[number];
/** What a running conversation can take mid-run. */
export const MIDWAY = ['steer', 'model', 'effort', 'approvals'] as const;
export type Midway = (typeof MIDWAY)[number];
/** How a prompt goes to a run: start a turn, after the running turn, or into it. */
export type Delivery = 'start' | 'queue' | 'steer';

/** What a driven client may do without asking. */
export const APPROVAL_POLICIES = ['ask', 'edits', 'auto'] as const;
export type ApprovalPolicy = (typeof APPROVAL_POLICIES)[number];

/** A run's settings; each null is the client's own. */
export interface RunSettings {
  model: ModelId | null;
  effort: Stored<Effort> | null;
  approvals: Stored<ApprovalPolicy> | null;
}
/** Settings as chosen in this UI; each null is the client's own. */
export interface ChosenSettings {
  model: ModelId | null;
  effort: Effort | null;
  approvals: ApprovalPolicy | null;
}
const runSettings = object<RunSettings, WireRunSettings>({
  model: nullable(branded<'ModelId'>()),
  effort: nullable(label(EFFORTS)),
  approvals: nullable(label(APPROVAL_POLICIES)),
});
export type Decision = (typeof decisions)[number];

/** What an approval asks the operator to allow. */
export type ApprovalAsk =
  | { kind: 'command'; command: string | null; cwd: string | null; reason: string | null }
  | { kind: 'file_change'; reason: string | null }
  | { kind: 'tool'; tool: string; detail: string | null }
  /** Questions to answer with replies, or decline. */
  | { kind: 'questions'; title: string | null; questions: Question[] }
  | { kind: 'unrecognized'; text: string };

export interface Waiting {
  approval: ApprovalId;
  ask: ApprovalAsk;
}

/** A run's lifecycle as the daemon holds it now. */
export type RunState =
  | { phase: 'starting' }
  | { phase: 'idle'; thread: NativeThreadId }
  | {
      phase: 'working';
      thread: NativeThreadId;
      turn: NativeTurnId | null;
      approvals: Waiting[];
      /** The operator's next prompt, sent as the turn ends. */
      queued: string | null;
    }
  | { phase: 'stopping'; cause: Stored<StopCause> }
  | { phase: 'ended'; end: Stored<RunEnd>; exit: ProcessExit | null }
  /** A phase a newer daemon reports. */
  | { phase: 'unrecognized'; text: string };

export interface RunSnapshot {
  run: RunId;
  /** The session the run serves. */
  session: SessionId;
  harness: Stored<Harness>;
  workdir: string;
  state: RunState;
  settings: RunSettings;
}

/** Where an item's action stands; a status a newer daemon names is kept as text. */
/** Where an item's action stands, pinned to `ren-harness`'s `drive::ItemStatus`; another is the client's own. */
export const itemStatuses = ['in_progress', 'completed', 'failed', 'declined'] as const;
export type ItemStatus = (typeof itemStatuses)[number] | (string & {});
/** The kinds of items Ren reads, pinned to `ren-harness`'s `drive::ItemKind`; another is the client's own. */
export const itemKinds = [
  'user_message',
  'agent_message',
  'reasoning',
  'plan',
  'command_execution',
  'file_change',
  'mcp_tool_call',
  'web_search',
] as const;
export type ItemKind = (typeof itemKinds)[number] | (string & {});

/** What the daemon read from an item, the same for every client. */
export type ItemRead =
  | { as: 'prompt'; text: string; images: AttachmentRead[] }
  | { as: 'reply'; text: string }
  | { as: 'reasoning' }
  | { as: 'plan'; text: string }
  | {
      as: 'command';
      command: string;
      intent: CommandIntent;
      status: ItemStatus;
      output: string | null;
      exit_code: number | null;
    }
  | { as: 'file_change'; paths: string[]; status: ItemStatus }
  /** Another tool: its name as the client gave it, and what it does as Ren
   * reads it (none for items stored before Ren read them). */
  | { as: 'tool'; name: string; effects: ToolEffect[]; status: ItemStatus }
  /** Questions the client asked, with the answer it reported for each, by key. */
  | {
      as: 'questions';
      questions: Question[];
      answers: Partial<Record<QuestionKey, string>>;
      status: ItemStatus;
    }
  /** Nothing read, or a reading a newer daemon gives: shown by the item's kind. */
  | { as: 'unread' };

/** One item of a turn: its kind and what the daemon read of it. */
export interface DriveItem {
  id: ItemId;
  kind: ItemKind;
  read: ItemRead;
}

export type TurnEnd =
  | { kind: 'completed' }
  | { kind: 'interrupted' }
  | { kind: 'failed'; message: string | null }
  | { kind: 'unknown'; status: string };

/** Every kind of event the daemon relays, pinned to `ren-harness`'s `drive::DriveEventKind`. */
export const driveEventKinds = [
  'ready',
  'turn_started',
  'delta',
  'item',
  'approval',
  'approval_resolved',
  'turn_ended',
  'error',
  'refused',
  'unsupported',
  'unknown',
  'unreadable',
] as const;
export type DriveEventKind = (typeof driveEventKinds)[number];

/** What the agent reported, as the daemon relays it. Not stored. */
export type DriveEvent =
  | { kind: 'ready'; thread: NativeThreadId }
  | { kind: 'turn_started'; turn: NativeTurnId }
  | { kind: 'delta'; item: ItemId; text: string }
  | { kind: 'item'; phase: 'started' | 'completed'; item: DriveItem }
  | { kind: 'approval'; approval: ApprovalId; ask: ApprovalAsk }
  | { kind: 'approval_resolved'; approval: ApprovalId }
  | { kind: 'turn_ended'; turn: NativeTurnId; end: TurnEnd }
  | { kind: 'error'; message: string; retry: 'will_retry' | 'final' }
  | { kind: 'refused'; request: string; message: string }
  | { kind: 'unsupported'; method: string }
  /** Unknown, unreadable, or newer events: shown by their kind only. */
  | { kind: 'other'; name: string };

export type RunUpdate =
  { kind: 'state'; run: RunId; state: RunState } | { kind: 'event'; run: RunId; event: DriveEvent };

/** A run as recorded: its start, opening, and end. */
export interface RecordedRun {
  id: RunId;
  harness: Stored<Harness>;
  workdir: string;
  started_at: UnixMillis;
  thread: NativeThreadId | null;
  /** The recording its model traffic was recorded under, once a call was. */
  recording: RecordingId | null;
  ended_at: UnixMillis | null;
  end: Stored<RunEnd> | null;
  exit: ProcessExit | null;
  /** When the operator archived it; null when it is not. */
  archived_at: UnixMillis | null;
  /** Its settings as started; null is the client's own (or not recorded then). */
  model: ModelId | null;
  effort: Stored<Effort> | null;
  approvals: Stored<ApprovalPolicy> | null;
}

/** What an archive or delete applies to. */
export type HistorySubject = { kind: 'recording'; id: RecordingId } | { kind: 'run'; id: RunId };

/** What a history edit does. */
export type HistoryChange = 'archive' | 'unarchive' | 'delete';

const optionalString: Decoder<string | null, string | null | undefined> = (value, path) =>
  value === undefined || value === null ? null : string(value, path);

/** A queued prompt's text: text alone, or with images, `{text, images}`. */
const queuedText: Decoder<string | null, WireRecordedPrompt | null | undefined> = (value, path) =>
  isRecord(value) ? string(value.text, `${path}.text`) : optionalString(value, path);

const integer: Decoder<number, number> = (value, path) => {
  if (typeof value !== 'number' || !Number.isInteger(value))
    throw new ShapeError(path, 'an integer');
  return value;
};

type AskOf<K> = Extract<ApprovalAsk, { kind: K }>;
type SentAsk<K> = Variant<WireApprovalAsk, 'kind', K>;
const approvalAsk = tagged<ApprovalAsk, WireApprovalAsk, 'kind'>(
  'kind',
  {
    command: object<AskOf<'command'>, SentAsk<'command'>>({
      kind: tagValue('command'),
      command: optionalString,
      cwd: optionalString,
      reason: optionalString,
    }),
    file_change: object<AskOf<'file_change'>, SentAsk<'file_change'>>({
      kind: tagValue('file_change'),
      reason: optionalString,
    }),
    tool: object<AskOf<'tool'>, SentAsk<'tool'>>({
      kind: tagValue('tool'),
      tool: string,
      detail: optionalString,
    }),
    questions: object<AskOf<'questions'>, SentAsk<'questions'>>({
      kind: tagValue('questions'),
      title: optionalString,
      questions: array(Questions.decode),
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { kind: 'unrecognized', text: String(value.kind) };
  },
);

const waiting = object<Waiting, WireWaiting>({
  approval: branded<'ApprovalId'>(),
  ask: approvalAsk,
});

type PhaseOf<K> = Extract<RunState, { phase: K }>;
type SentPhase<K> = Variant<WireRunState, 'phase', K>;
const runState = tagged<RunState, WireRunState, 'phase'>(
  'phase',
  {
    starting: object<PhaseOf<'starting'>, SentPhase<'starting'>>({ phase: tagValue('starting') }),
    idle: object<PhaseOf<'idle'>, SentPhase<'idle'>>({
      phase: tagValue('idle'),
      thread: branded<'NativeThreadId'>(),
    }),
    working: object<PhaseOf<'working'>, SentPhase<'working'>>({
      phase: tagValue('working'),
      thread: branded<'NativeThreadId'>(),
      turn: absentAs(nullable(branded<'NativeTurnId'>()), null),
      approvals: absentAs(array(waiting), []),
      queued: queuedText,
    }),
    stopping: object<PhaseOf<'stopping'>, SentPhase<'stopping'>>({
      phase: tagValue('stopping'),
      cause: label(stopCauses),
    }),
    ended: object<PhaseOf<'ended'>, SentPhase<'ended'>>({
      phase: tagValue('ended'),
      end: label(runEnds),
      exit: absentAs(nullable(processExit), null),
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { phase: 'unrecognized', text: String(value.phase) };
  },
);

/** One part of a command as its client read it (Codex's `commandActions`). */
export type CommandPart =
  | { part: 'read'; command: string; path: string; name: string }
  | { part: 'list_files'; command: string; path: string | null }
  | { part: 'search'; command: string; query: string | null; path: string | null }
  /** A part its client could not read, or of a kind this UI does not know. */
  | { part: 'unknown'; command: string | null };

/** What a command is for, in its client's terms. */
export type CommandIntent =
  | { said: 'unsaid' }
  | { said: 'described'; text: string }
  | { said: 'parts'; parts: CommandPart[] };

type PartOf<K> = Extract<CommandPart, { part: K }>;
type SentPart<K> = Variant<WireCommandPart, 'part', K>;
/** A part Ren could not read: the command its client gave, when it gave one. */
const unreadPart = (value: unknown): CommandPart => {
  const raw = isRecord(value) ? (isRecord(value.raw) ? value.raw.command : value.command) : null;
  return { part: 'unknown', command: typeof raw === 'string' ? raw : null };
};
const commandPart = tagged<CommandPart, WireCommandPart, 'part'>(
  'part',
  {
    read: object<PartOf<'read'>, SentPart<'read'>>({
      part: tagValue('read'),
      command: string,
      path: string,
      name: string,
    }),
    list_files: object<PartOf<'list_files'>, SentPart<'list_files'>>({
      part: tagValue('list_files'),
      command: string,
      path: optionalString,
    }),
    search: object<PartOf<'search'>, SentPart<'search'>>({
      part: tagValue('search'),
      command: string,
      query: optionalString,
      path: optionalString,
    }),
    unknown: object<PartOf<'unknown'>, SentPart<'unknown'>>({
      part: tagValue('unknown'),
      command: string,
    }),
    unrecognized: unreadPart,
  },
  unreadPart,
);

type IntentOf<K> = Extract<CommandIntent, { said: K }>;
type SentIntent<K> = Variant<WireCommandIntent, 'said', K>;
const commandIntent = tagged<CommandIntent, WireCommandIntent, 'said'>(
  'said',
  {
    unsaid: object<IntentOf<'unsaid'>, SentIntent<'unsaid'>>({ said: tagValue('unsaid') }),
    described: object<IntentOf<'described'>, SentIntent<'described'>>({
      said: tagValue('described'),
      text: string,
    }),
    parts: object<IntentOf<'parts'>, SentIntent<'parts'>>({
      said: tagValue('parts'),
      parts: array(commandPart),
    }),
  },
  () => ({ said: 'unsaid' }),
);

/** Each question's answer, by its key. */
const answers: Decoder<Partial<Record<QuestionKey, string>>, Partial<Record<string, string>>> = (
  value,
  path,
) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  const read: Partial<Record<QuestionKey, string>> = {};
  for (const [key, answer] of Object.entries(value)) {
    read[key as QuestionKey] = string(answer, `${path}.${key}`);
  }
  return read;
};

type ReadOf<K> = Extract<ItemRead, { as: K }>;
type SentRead<K> = Variant<WireItemRead, 'as', K>;
const itemRead = tagged<ItemRead, WireItemRead, 'as'>(
  'as',
  {
    prompt: object<ReadOf<'prompt'>, SentRead<'prompt'>>({
      as: tagValue('prompt'),
      text: string,
      images: absentAs(array(attachmentRead), []),
    }),
    reply: object<ReadOf<'reply'>, SentRead<'reply'>>({ as: tagValue('reply'), text: string }),
    reasoning: object<ReadOf<'reasoning'>, SentRead<'reasoning'>>({ as: tagValue('reasoning') }),
    plan: object<ReadOf<'plan'>, SentRead<'plan'>>({ as: tagValue('plan'), text: string }),
    command: object<ReadOf<'command'>, SentRead<'command'>>({
      as: tagValue('command'),
      command: string,
      intent: absentAs(commandIntent, { said: 'unsaid' }),
      status: string,
      output: optionalString,
      exit_code: absentAs(nullable(integer), null),
    }),
    file_change: object<ReadOf<'file_change'>, SentRead<'file_change'>>({
      as: tagValue('file_change'),
      paths: array(string),
      status: string,
    }),
    tool: object<ReadOf<'tool'>, SentRead<'tool'>>({
      as: tagValue('tool'),
      name: string,
      effects: absentAs(array(toolEffect), []),
      status: string,
    }),
    questions: object<ReadOf<'questions'>, SentRead<'questions'>>({
      as: tagValue('questions'),
      questions: array(Questions.decode),
      answers: absentAs(answers, {}),
      status: string,
    }),
    unread: object<ReadOf<'unread'>, SentRead<'unread'>>({ as: tagValue('unread') }),
  },
  () => ({ as: 'unread' }),
);

const runSnapshot = object<RunSnapshot, WireRunSnapshot>({
  run: RunId.decode,
  session: SessionId.decode,
  harness: label(harnesses),
  workdir: string,
  state: runState,
  settings: runSettings,
});

type EndOf<K> = Extract<TurnEnd, { kind: K }>;
type SentEnd<K> = Variant<WireTurnEnd, 'kind', K>;
const turnEnd = tagged<TurnEnd, WireTurnEnd, 'kind'>(
  'kind',
  {
    completed: object<EndOf<'completed'>, SentEnd<'completed'>>({ kind: tagValue('completed') }),
    interrupted: object<EndOf<'interrupted'>, SentEnd<'interrupted'>>({
      kind: tagValue('interrupted'),
    }),
    failed: object<EndOf<'failed'>, SentEnd<'failed'>>({
      kind: tagValue('failed'),
      message: optionalString,
    }),
    unknown: object<EndOf<'unknown'>, SentEnd<'unknown'>>({
      kind: tagValue('unknown'),
      status: string,
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { kind: 'unknown', status: String(value.status ?? value.kind) };
  },
);

/** An item's phase; anything but `completed` reads as started. */
const itemPhase: Decoder<'started' | 'completed', WireItemPhase> = (value) =>
  value === 'completed' ? 'completed' : 'started';
/** Whether the agent retries after an error; anything but `will_retry` is final. */
const retry: Decoder<'will_retry' | 'final', WireRetry> = (value) =>
  value === 'will_retry' ? 'will_retry' : 'final';
type EventOf<K> = Extract<DriveEvent, { kind: K }>;
type SentEvent<K> = Variant<WireDriveEvent, 'kind', K>;
const driveEvent = tagged<DriveEvent, WireDriveEvent, 'kind'>(
  'kind',
  {
    ready: object<EventOf<'ready'>, SentEvent<'ready'>>({
      kind: tagValue('ready'),
      thread: branded<'NativeThreadId'>(),
    }),
    turn_started: object<EventOf<'turn_started'>, SentEvent<'turn_started'>>({
      kind: tagValue('turn_started'),
      turn: branded<'NativeTurnId'>(),
    }),
    delta: object<EventOf<'delta'>, SentEvent<'delta'>>({
      kind: tagValue('delta'),
      item: branded<'ItemId'>(),
      text: string,
    }),
    item: object<EventOf<'item'>, SentEvent<'item'>>({
      kind: tagValue('item'),
      phase: itemPhase,
      item: object<DriveItem, WireDriveItem>({
        id: branded<'ItemId'>(),
        kind: string,
        read: itemRead,
      }),
    }),
    approval: object<EventOf<'approval'>, SentEvent<'approval'>>({
      kind: tagValue('approval'),
      approval: branded<'ApprovalId'>(),
      ask: approvalAsk,
    }),
    approval_resolved: object<EventOf<'approval_resolved'>, SentEvent<'approval_resolved'>>({
      kind: tagValue('approval_resolved'),
      approval: branded<'ApprovalId'>(),
    }),
    turn_ended: object<EventOf<'turn_ended'>, SentEvent<'turn_ended'>>({
      kind: tagValue('turn_ended'),
      turn: branded<'NativeTurnId'>(),
      end: turnEnd,
    }),
    error: object<EventOf<'error'>, SentEvent<'error'>>({
      kind: tagValue('error'),
      message: string,
      retry,
    }),
    refused: object<EventOf<'refused'>, SentEvent<'refused'>>({
      kind: tagValue('refused'),
      request: string,
      message: string,
    }),
    unsupported: object<EventOf<'unsupported'>, SentEvent<'unsupported'>>({
      kind: tagValue('unsupported'),
      method: string,
    }),
    // Unknown and unreadable lines are shown by their kind only.
    unknown: () => ({ kind: 'other', name: 'unknown' }),
    unreadable: () => ({ kind: 'other', name: 'unreadable' }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { kind: 'other', name: string(value.kind, `${path}.kind`) };
  },
);

/** One `run` event of the update feed. */
type UpdateOf<K> = Extract<RunUpdate, { kind: K }>;
type SentUpdate<K> = Variant<WireRunUpdate, 'kind', K>;
export const runUpdate = tagged<RunUpdate, WireRunUpdate, 'kind'>(
  'kind',
  {
    state: object<UpdateOf<'state'>, SentUpdate<'state'>>({
      kind: tagValue('state'),
      run: RunId.decode,
      state: runState,
    }),
    event: object<UpdateOf<'event'>, SentUpdate<'event'>>({
      kind: tagValue('event'),
      run: RunId.decode,
      event: driveEvent,
    }),
  },
  (_value, path) => {
    throw new ShapeError(`${path}.kind`, 'state or event');
  },
);

const recordedRun = object<RecordedRun, WireRunView>({
  id: RunId.decode,
  harness: label(harnesses),
  workdir: string,
  started_at: brandedCount<'UnixMillis'>(),
  thread: nullable(branded<'NativeThreadId'>()),
  recording: nullable(RecordingId.decode),
  ended_at: nullable(brandedCount<'UnixMillis'>()),
  end: nullable(label(runEnds)),
  exit: nullable(processExit),
  archived_at: nullable(brandedCount<'UnixMillis'>()),
  model: nullable(branded<'ModelId'>()),
  effort: nullable(label(EFFORTS)),
  approvals: nullable(label(APPROVAL_POLICIES)),
});

const liveRuns = versioned(
  object<{ runs: RunSnapshot[] }, WireRunsBody>({ runs: array(runSnapshot) }),
);

/** Whether a prompt has gone out in a session: a draft's client can still change. */
export const SESSION_PHASES = ['draft', 'active'] as const;
export type SessionPhase = (typeof SESSION_PHASES)[number];
const shelves = ['open', 'archived'] as const;

/** Where a session stands: on the open list or the archived one, and in what phase. */
export interface SessionState {
  shelf: Stored<(typeof shelves)[number]>;
  phase: Stored<SessionPhase>;
}

/** A session as the daemon serves it now. */
export interface ServedSession {
  session: SessionId;
  harness: Stored<Harness>;
  workdir: string;
  /** What its next turn runs on. */
  settings: RunSettings;
  state: SessionState;
  /** Its newest process, whose state the run feed carries. */
  process: RunId | null;
  /** Whether its client takes images with a prompt. */
  images: Stored<ImageTaking>;
}

/** A session as the store holds it, with its runs newest first. */
export interface StoredSession {
  id: SessionId;
  harness: Stored<Harness>;
  workdir: string;
  /** The project it works for, by id; null outside every project. */
  project: string | null;
  created_at: UnixMillis;
  model: ModelId | null;
  effort: Stored<Effort> | null;
  approvals: Stored<ApprovalPolicy> | null;
  archived_at: UnixMillis | null;
  /** When its first turn started; null while it is a draft. */
  first_turn_at: UnixMillis | null;
  /** The first line of its first prompt, cut short; null until one is recorded. */
  first_prompt: string | null;
  runs: RecordedRun[];
}

/** A worktree of a project's repository Ren made for one session, on a
 * branch of its own, with the ports and data folder its runs are given. */
export interface Worktree {
  /** `<repo>-<slot>`, its folder's name. */
  name: string;
  data: string;
  /** Its ports, first and last. */
  ports: [number, number];
}

/** One session: as stored and as served; either may be missing for a moment. */
export interface SessionBody {
  session: SessionId;
  stored: StoredSession | null;
  served: ServedSession | null;
  /** The worktree it works in; null in any other folder. */
  worktree: Worktree | null;
}

const servedSession = object<ServedSession, WireSessionSnapshot>({
  session: SessionId.decode,
  harness: label(harnesses),
  workdir: string,
  settings: runSettings,
  state: object<SessionState, WireSessionState>({
    shelf: label(shelves),
    phase: label(SESSION_PHASES),
  }),
  process: nullable(RunId.decode),
  // A daemon that does not say takes no images.
  images: (value, path) => (value === undefined ? 'never' : label(IMAGE_TAKING)(value, path)),
});

const storedSession = object<StoredSession, WireSessionView>({
  id: SessionId.decode,
  harness: label(harnesses),
  workdir: string,
  project: nullable(string),
  created_at: brandedCount<'UnixMillis'>(),
  model: nullable(branded<'ModelId'>()),
  effort: nullable(label(EFFORTS)),
  approvals: nullable(label(APPROVAL_POLICIES)),
  archived_at: nullable(brandedCount<'UnixMillis'>()),
  first_turn_at: nullable(brandedCount<'UnixMillis'>()),
  first_prompt: nullable(string),
  runs: array(recordedRun),
});

const ports = (value: unknown, path: string): [number, number] => {
  if (!Array.isArray(value) || value.length !== 2) throw new ShapeError(path, 'two ports');
  return [count(value[0], `${path}[0]`), count(value[1], `${path}[1]`)];
};
const sessionBody = object<SessionBody, WireSessionBody>({
  session: SessionId.decode,
  stored: nullable(storedSession),
  served: nullable(servedSession),
  worktree: nullable(object<Worktree, WireWorktree>({ name: string, data: string, ports })),
});
const sessionList = versioned(
  object<{ sessions: SessionBody[] }, WireSessionsBody>({ sessions: array(sessionBody) }),
);
const oneSession = versioned(
  object<{ session: SessionBody }, WireOneSession>({ session: sessionBody }),
);
const createdSession = versioned(
  object<{ session: SessionId }, WireCreated>({ session: SessionId.decode }),
);
/** A place's repository, as its `HEAD` reads. */
export type PlaceGit =
  | { kind: 'branch'; name: string }
  | { kind: 'detached'; commit: string }
  | { kind: 'unreadable' }
  | { kind: 'none' };

export const placeKinds = ['project', 'other', 'scratch'] as const;
/** What kind of place a run can start in. */
export type PlaceKind = (typeof placeKinds)[number];

/**
 * A place a run can start in: a project under a run root (a git repository,
 * or a root's own folder), a folder of another configured place, or the
 * folder of an earlier scratch run.
 */
export interface RunPlace {
  path: string;
  name: string;
  /** The path as the operator reads it: `~/…` under their home. */
  shown: string;
  kind: Stored<PlaceKind>;
  git: PlaceGit;
}

/** Where runs can start, and the folder scratch runs get theirs in. */
export interface RunPlaces {
  places: RunPlace[];
  /** Null when runs cannot start in scratch on the daemon's host. */
  scratch: string | null;
}

type GitOf<K> = Extract<PlaceGit, { kind: K }>;
type SentGit<K> = Variant<WirePlaceGit, 'kind', K>;
const placeGit = tagged<PlaceGit, WirePlaceGit, 'kind'>(
  'kind',
  {
    branch: object<GitOf<'branch'>, SentGit<'branch'>>({ kind: tagValue('branch'), name: string }),
    detached: object<GitOf<'detached'>, SentGit<'detached'>>({
      kind: tagValue('detached'),
      commit: string,
    }),
    unreadable: object<GitOf<'unreadable'>, SentGit<'unreadable'>>({
      kind: tagValue('unreadable'),
    }),
    none: object<GitOf<'none'>, SentGit<'none'>>({ kind: tagValue('none') }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { kind: 'unreadable' };
  },
);
const runPlaces = versioned(
  object<RunPlaces, WirePlaces>({
    places: array(
      object<RunPlace, WirePlace>({
        path: string,
        name: string,
        shown: string,
        kind: label(placeKinds),
        git: placeGit,
      }),
    ),
    scratch: nullable(string),
  }),
);
const recordedRuns = versioned(
  object<{ runs: RecordedRun[] }, { runs: WireRunView[] }>({ runs: array(recordedRun) }),
);

export const clientStatuses = ['ready', 'not_installed'] as const;
/** Whether a run can start with a client on the daemon's host. */
export type ClientStatus = (typeof clientStatuses)[number];

/** A client, and whether a run can start with it on the daemon's host. */
export interface RunClient {
  harness: Stored<Harness>;
  status: Stored<ClientStatus>;
  /** The approval policies it can run under; none for a client that never asks. */
  approvals: Stored<ApprovalPolicy>[];
  /** What a running conversation takes mid-run. */
  midway: Stored<Midway>[];
  /** Its sign-in on the daemon's host, as its status command last said. */
  sign_in: SignInState;
  /** How Ren can sign it in, first the usual way; none where it cannot. */
  sign_in_methods: Stored<SignInMethod>[];
}

const runClients = versioned(
  object<{ clients: RunClient[] }, WireClientsBody>({
    clients: array(
      object<RunClient, WireClient>({
        harness: label(harnesses),
        status: label(clientStatuses),
        approvals: array(label(APPROVAL_POLICIES)),
        midway: array(label(MIDWAY)),
        sign_in: (value, path) =>
          value === undefined ? { kind: 'not_checked' } : signInState(value, path),
        sign_in_methods: (value, path) =>
          value === undefined ? [] : array(signInMethod)(value, path),
      }),
    ),
  }),
);
/** A model, named as its client names it (`--model`, or Pi's `provider/id`). */
export type ModelId = Brand<string, 'ModelId'>;
export const modelChoices = ['default', 'other'] as const;

/** A model a client offers, as the client itself lists it. */
export interface RunModel {
  id: ModelId;
  name: string;
  description: string | null;
  /** The provider serving it, where the client offers several (Pi). */
  provider: string | null;
  /** Whether a run gets it when no model is chosen. */
  choice: Stored<(typeof modelChoices)[number]>;
  /** The efforts it takes, least first; empty when it offers no choice. */
  efforts: Stored<Effort>[];
  /** The effort it runs at when none is chosen, when the client says. */
  effort: Stored<Effort> | null;
}

const runModels = versioned(
  object<{ models: RunModel[] }, WireModelsBody>({
    models: array(
      object<RunModel, WireModel>({
        id: branded<'ModelId'>(),
        name: string,
        description: nullable(string),
        provider: nullable(string),
        choice: label(modelChoices),
        efforts: array(label(EFFORTS)),
        effort: nullable(label(EFFORTS)),
      }),
    ),
  }),
);

/** What a command is to its client. */
export const COMMAND_KINDS = ['command', 'skill', 'template'] as const;
export type CommandKind = (typeof COMMAND_KINDS)[number];
/** How a prompt names a command: `/name` (Claude Code, Pi) or `$name` (Codex's skills). */
export const SIGILS = ['/', '$'] as const;
export type Sigil = (typeof SIGILS)[number];

/** A command, skill, or prompt template a run's client offers, as it listed it. */
export interface ClientCommand {
  name: string;
  /** Empty when the client gives none. */
  description: string;
  /** What it takes after its name, when the client says. */
  hint: string | null;
  kind: Stored<CommandKind>;
  sigil: Stored<Sigil>;
  /** What a prompt writes to run it: its sigil, then its name. */
  invocation: string;
}

const clientCommands = versioned(
  object<{ commands: ClientCommand[] }, WireCommandsBody>({
    commands: array(
      object<ClientCommand, WireClientCommand>({
        name: string,
        description: string,
        hint: nullable(string),
        kind: label(COMMAND_KINDS),
        sigil: label(SIGILS),
        invocation: string,
      }),
    ),
  }),
);

export const PATH_KINDS = ['file', 'directory'] as const;
export type PathKind = (typeof PATH_KINDS)[number];
/** A path of a run's folder that matched a search, with the characters (by index) that matched. */
export interface FileMatch {
  /** Relative to the run's folder, `/`-separated. */
  path: string;
  kind: Stored<PathKind>;
  positions: number[];
}

const fileMatches = versioned(
  object<{ files: FileMatch[] }, WireFilesBody>({
    files: array(
      object<FileMatch, WireFileMatch>({
        path: string,
        kind: label(PATH_KINDS),
        positions: array(count),
      }),
    ),
  }),
);

const versionedRecordedRun = versioned(recordedRun);
const storedConversation = versioned(
  object<StoredConversation, WireConversationView>({
    entries: array(
      object<StoredConversation['entries'][number], WireRunEntryView>({
        run: RunId.decode,
        seq: count,
        event: (value, path) => driveEvent(value, path),
      }),
    ),
    earlier: boolean,
  }),
);

/** Why the daemon or this UI server refused a run command, by its error code. */
export class RunCommandError extends Error {
  readonly status: number;
  readonly code: Stored<Refusal> | null;
  constructor(status: number, code: Stored<Refusal> | null) {
    super(code === null ? `The command was refused (${status}).` : Refusals.say(code));
    this.status = status;
    this.code = code;
  }
}

/** The clients the daemon can drive, in the order a draft offers them:
 * Claude Code, the default, first. */
export const DRIVABLE = [
  'claude_code',
  'codex',
  'pi',
  'opencode',
] as const satisfies readonly Harness[];
export type Drivable = (typeof DRIVABLE)[number];

/** One read the page starts with: what was read, or why it was not. */
export type Read<T> = { state: 'read'; value: T } | { state: 'failed'; reason: string };

/** Everything the Sessions page shows before it starts following the feed. */
export interface RunsStart {
  live: Read<RunSnapshot[]>;
  places: Read<RunPlaces>;
  clients: Read<RunClient[]>;
  recorded: Read<RecordedRun[]>;
  archived: Read<RecordedRun[]>;
  sessions: Read<SessionBody[]>;
  archivedSessions: Read<SessionBody[]>;
}

/** Run commands and reads through a capability-supplied `fetch`. */
export class RunsClient {
  readonly #fetch: typeof fetch;
  constructor(fetcher: typeof fetch = fetch) {
    this.#fetch = fetcher;
  }

  async live(signal?: AbortSignal): Promise<RunSnapshot[]> {
    return (await this.#send('GET', '/control/runs', undefined, liveRuns, signal)).runs;
  }
  /**
   * Everything the Sessions page starts with, read at once; each read that
   * fails says why, and the others stand.
   */
  async opening(): Promise<RunsStart> {
    const read = async <T>(reading: Promise<T>): Promise<Read<T>> => {
      try {
        return { state: 'read', value: await reading };
      } catch (error) {
        return {
          state: 'failed',
          reason: error instanceof Error ? error.message : 'The read failed.',
        };
      }
    };
    const [live, places, clients, recorded, archived, sessions, archivedSessions] =
      await Promise.all([
        read(this.live()),
        read(this.places()),
        read(this.clients()),
        read(this.recorded(100)),
        read(this.recorded(100, 'archived')),
        read(this.sessions()),
        read(this.sessions('archived')),
      ]);
    return {
      live,
      places,
      clients,
      recorded,
      archived,
      sessions,
      archivedSessions,
    };
  }
  /** Where runs can start, and the scratch folder. */
  async places(signal?: AbortSignal): Promise<RunPlaces> {
    return this.#send('GET', '/control/runs/places', undefined, runPlaces, signal);
  }
  /** Every client, and whether a run can start with it on the daemon's host. */
  async clients(signal?: AbortSignal): Promise<RunClient[]> {
    return (await this.#send('GET', '/control/runs/clients', undefined, runClients, signal))
      .clients;
  }
  /** Check `harness`'s sign-in now, with its own status command. */
  async checkSignIn(harness: Drivable): Promise<SignInState> {
    const path = `/control/runs/clients/${harness}/sign-in/check`;
    return (await this.#send('POST', path, {}, checkedSignIn, undefined, 25_000)).sign_in;
  }
  /** Start `harness`'s own login; answers once it shows its link. */
  async startSignIn(harness: Drivable, method: SignInMethod): Promise<Login> {
    const path = `/control/runs/clients/${harness}/sign-in`;
    return (
      await this.#send(
        'POST',
        path,
        { method } satisfies WireSignInStart,
        startedSignIn,
        undefined,
        28_000,
      )
    ).login;
  }
  /** Hand `harness`'s login what it waits for (a code, a key, or an address); answers once it ended and was checked. */
  async signInAnswer(harness: Drivable, answer: string): Promise<LoginOutcome> {
    const path = `/control/runs/clients/${harness}/sign-in/answer`;
    return (
      await this.#send(
        'POST',
        path,
        { answer } satisfies WireLoginAnswer,
        finishedSignIn,
        undefined,
        28_000,
      )
    ).outcome;
  }
  /** Wait up to about 20 seconds for `harness`'s login to end and be checked. */
  async waitSignIn(harness: Drivable): Promise<Waited> {
    const path = `/control/runs/clients/${harness}/sign-in/wait`;
    return (await this.#send('POST', path, {}, waitedSignIn, undefined, 28_000)).waited;
  }
  /** Stop `harness`'s running login. */
  async cancelSignIn(harness: Drivable): Promise<void> {
    const path = `/control/runs/clients/${harness}/sign-in/cancel`;
    await this.#send('POST', path, {}, null);
  }
  /** The models `harness` offers, as it lists them on the daemon's host. */
  async models(harness: Drivable, signal?: AbortSignal): Promise<RunModel[]> {
    const path = `/control/runs/clients/${harness}/models`;
    return (await this.#send('GET', path, undefined, runModels, signal, 30_000)).models;
  }
  /** Sessions on `shelf`, most recently active first; with `project`,
   * only those working for the project of that name. */
  async sessions(
    shelf: Shelf = 'current',
    signal?: AbortSignal,
    project?: string,
  ): Promise<SessionBody[]> {
    const path = `/control/sessions?${query<WireListRequest>({ shelf, project })}`;
    return (await this.#send('GET', path, undefined, sessionList, signal)).sessions;
  }
  async session(session: SessionId, signal?: AbortSignal): Promise<SessionBody> {
    const path = `/control/sessions/${session}`;
    return (await this.#send('GET', path, undefined, oneSession, signal)).session;
  }
  /** A new draft in `dir` (null: a scratch folder of its own), or with `checkout` set to `worktree`, in a worktree of its own made from the repository in `dir`; its client starts with its first prompt. */
  async createSession(
    harness: Drivable,
    dir: string | null,
    settings: Partial<ChosenSettings> = {},
    checkout: 'folder' | 'worktree' = 'folder',
  ): Promise<SessionId> {
    const body = {
      harness,
      ...(dir ? { dir } : {}),
      ...(dir && checkout === 'worktree' ? { checkout } : {}),
      ...(settings.model ? { model: settings.model } : {}),
      ...(settings.effort ? { effort: settings.effort } : {}),
      ...(settings.approvals ? { approvals: settings.approvals } : {}),
    } satisfies WireCreateRequest;
    return (await this.#send('POST', '/control/sessions', body, createdSession)).session;
  }
  /** A session for the registered `project`, in a worktree of its own in
   * the project's repository. */
  async createProjectSession(harness: Drivable, project: string): Promise<SessionId> {
    const body = {
      harness,
      project,
      checkout: 'worktree',
    } satisfies WireCreateRequest;
    return (await this.#send('POST', '/control/sessions', body, createdSession)).session;
  }
  /** One command to a session: through its process, or a new one for a prompt. */
  async toSession(
    session: SessionId,
    command:
      | { kind: 'prompt'; text: string; delivery: Delivery; attachments?: AttachmentId[] }
      | ({ kind: 'settings' } & ChosenSettings)
      | { kind: 'client'; harness: Drivable }
      /** A draft's folder; null: a new scratch folder of its own. */
      | { kind: 'folder'; dir: string | null }
      | { kind: 'answer'; approval: ApprovalId; decision: Decision }
      | { kind: 'reply'; approval: ApprovalId; request: WireReplyRequest }
      | { kind: 'unqueue' | 'interrupt' | 'stop' | 'archive' | 'unarchive' | 'delete' },
  ): Promise<void> {
    const base = `/control/sessions/${session}`;
    switch (command.kind) {
      case 'prompt':
        return this.#send(
          'POST',
          `${base}/prompt`,
          {
            text: command.text,
            delivery: command.delivery,
            ...(command.attachments?.length ? { attachments: command.attachments } : {}),
          } satisfies WirePromptRequest,
          null,
        );
      case 'settings': {
        const { model, effort, approvals } = command;
        return this.#send(
          'POST',
          `${base}/settings`,
          { model, effort, approvals } satisfies WireSettingsRequest,
          null,
        );
      }
      case 'client':
        return this.#send(
          'POST',
          `${base}/client`,
          { harness: command.harness } satisfies WireClientRequest,
          null,
        );
      case 'folder':
        return this.#send(
          'POST',
          `${base}/folder`,
          (command.dir === null ? {} : { dir: command.dir }) satisfies WireFolderRequest,
          null,
        );
      case 'answer':
        return this.#send(
          'POST',
          `${base}/approvals/${encodeURIComponent(command.approval)}`,
          { decision: command.decision } satisfies WireAnswerRequest,
          null,
        );
      case 'reply':
        return this.#send(
          'POST',
          `${base}/approvals/${encodeURIComponent(command.approval)}/replies`,
          command.request,
          null,
        );
      case 'unqueue':
      case 'interrupt':
      case 'stop':
      case 'archive':
      case 'unarchive':
      case 'delete':
        return this.#send('POST', `${base}/${command.kind}`, {}, null);
      default:
        return assertNever(command);
    }
  }
  /** Upload an image to the session, to go with a prompt. */
  async attach(
    session: SessionId,
    name: string,
    type: string,
    bytes: ArrayBuffer,
  ): Promise<{ id: AttachmentId; read: AttachmentRead }> {
    const response = await this.#fetch(
      `/control/sessions/${session}/attachments?${query<WireAttachRequest>({ name })}`,
      {
        method: 'POST',
        cache: 'no-store',
        headers: { accept: 'application/json', 'content-type': type },
        body: bytes,
        signal: AbortSignal.timeout(120_000),
      },
    );
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    if (!response.ok) {
      throw new RunCommandError(response.status, Refusals.read(parsed));
    }
    return uploaded(parsed, '').attachment;
  }
  /** The session's attachments, and whether each was sent. */
  async attachments(session: SessionId, signal?: AbortSignal): Promise<KeptAttachment[]> {
    const path = `/control/sessions/${session}/attachments`;
    return (await this.#send('GET', path, undefined, keptList, signal)).attachments;
  }
  /** Remove an attachment not sent yet. */
  async detach(session: SessionId, attachment: AttachmentId): Promise<void> {
    return this.#send(
      'DELETE',
      `/control/sessions/${session}/attachments/${attachment}`,
      undefined,
      null,
    );
  }
  /** What the session's client offers a prompt. */
  async sessionCommands(session: SessionId, signal?: AbortSignal): Promise<ClientCommand[]> {
    const path = `/control/sessions/${session}/commands`;
    return (await this.#send('GET', path, undefined, clientCommands, signal)).commands;
  }
  /** The paths of the session's folder that best match `typed`. */
  async sessionFiles(
    session: SessionId,
    typed: string,
    limit = 20,
    signal?: AbortSignal,
  ): Promise<FileMatch[]> {
    const search = query<WireFilesRequest>({ q: typed, limit });
    const path = `/control/sessions/${session}/files?${search}`;
    return (await this.#send('GET', path, undefined, fileMatches, signal)).files;
  }
  /**
   * Archive, unarchive, or delete what was recorded of a session or a run.
   * The daemon applies it after what it is recording; the live feed says
   * when.
   */
  async edit(subject: HistorySubject, change: HistoryChange): Promise<void> {
    const kind = subject.kind === 'recording' ? 'recordings' : 'runs';
    await this.#send('POST', `/control/history/${kind}/${subject.id}/${change}`, {}, null);
  }
  /** Recorded runs on `shelf`, newest first, from the read-only management API. */
  async recorded(
    limit = 100,
    shelf: Shelf = 'current',
    signal?: AbortSignal,
    project: string | null = null,
  ): Promise<RecordedRun[]> {
    const params = query<WireRawShelfQuery>({
      limit,
      shelf: shelf === 'archived' ? 'archived' : null,
      project,
    });
    return (await this.#send('GET', `/api/runs?${params}`, undefined, recordedRuns, signal)).runs;
  }
  /** A run's conversation, whole, as the daemon stored it. */
  async conversation(run: RunId, signal?: AbortSignal): Promise<StoredConversation> {
    return this.#send(
      'GET',
      // Only what Ren read of each item is shown, never the client's own
      // body, which holds every tool's input and output again.
      `/api/runs/${run}/conversation?${query<WireRawConversationQuery>({ bodies: 'omitted' })}`,
      undefined,
      storedConversation,
      signal,
    );
  }
  async recordedRun(run: RunId, signal?: AbortSignal): Promise<RecordedRun> {
    return this.#send('GET', `/api/runs/${run}`, undefined, versionedRecordedRun, signal);
  }

  async #send<T>(
    method: 'GET' | 'POST' | 'DELETE',
    path: string,
    body: unknown,
    decode: AnyDecoder<T> | null,
    signal?: AbortSignal,
    wait = 15_000,
  ): Promise<T> {
    const timeout = AbortSignal.timeout(wait);
    const response = await this.#fetch(path, {
      method,
      cache: 'no-store',
      headers:
        body === undefined
          ? { accept: 'application/json' }
          : { accept: 'application/json', 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    });
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    if (!response.ok) {
      throw new RunCommandError(response.status, Refusals.read(parsed));
    }
    if (decode === null) {
      if (!isRecord(parsed) || parsed.api_version !== API_VERSION) {
        throw new ShapeError('api_version', `${API_VERSION}`);
      }
      return undefined as T;
    }
    return decode(parsed, '');
  }
}

/** One entry of a run's conversation, built from its events. */
export type Entry =
  | { kind: 'prompt'; item: ItemId; text: string; images: AttachmentRead[] }
  | { kind: 'message'; item: ItemId; text: string; done: boolean }
  | { kind: 'reasoning'; item: ItemId; done: boolean }
  | {
      kind: 'command';
      item: ItemId;
      command: string;
      intent: CommandIntent;
      status: string;
      output: string | null;
      exit: number | null;
    }
  | { kind: 'file_change'; item: ItemId; paths: string[]; status: string }
  | { kind: 'tool'; item: ItemId; name: string; effects: ToolEffect[]; done: boolean }
  | {
      kind: 'questions';
      item: ItemId;
      questions: Question[];
      answers: Partial<Record<QuestionKey, string>>;
      status: string;
    }
  | { kind: 'turn_end'; turn: NativeTurnId; end: TurnEnd }
  | { kind: 'notice'; tone: 'warn' | 'danger'; text: string };

/** A run's conversation as the daemon stored it (`/api/runs/{id}/conversation`). */
export interface StoredConversation {
  entries: { run: RunId; seq: number; event: DriveEvent }[];
  /** Older entries were left out of the read. */
  earlier: boolean;
}

/**
 * A run's conversation, whole: read once from what the daemon stored of it
 * (every turn of the run, and of the runs before it on its thread), then kept
 * current by the run feed. Both say the same events, so they merge by what
 * each entry is: an item by its id, a turn's end by its turn, anything else by
 * what it says. Nothing is cut, and nothing gives way to a shorter copy.
 */
export class Transcript {
  readonly entries: Entry[];
  /** The turn the newest entries belong to, once the client named it. */
  readonly turn: NativeTurnId | null;
  /** Whether what the daemon stored has been read. */
  readonly read: boolean;
  /** Older entries were left out of the read. */
  readonly earlier: boolean;

  constructor(
    entries: Entry[] = [],
    turn: NativeTurnId | null = null,
    read = false,
    earlier = false,
  ) {
    this.entries = entries;
    this.turn = turn;
    this.read = read;
    this.earlier = earlier;
  }

  /** What the daemon stored, with what streamed in meanwhile kept. */
  load(stored: StoredConversation): Transcript {
    let loaded = new Transcript();
    for (const entry of stored.entries) loaded = loaded.apply(entry.event, entry.run);
    const entries = loaded.entries.slice();
    for (const live of this.entries) {
      const at = entries.findIndex((entry) => Transcript.same(entry, live));
      if (at === -1) entries.push(live);
      else if (Transcript.done(live) && !Transcript.done(entries[at]!)) entries[at] = live;
    }
    return new Transcript(entries, this.turn ?? loaded.turn, true, stored.earlier);
  }

  /** Every prompt sent, oldest first, to recall. */
  prompts(): string[] {
    return this.entries.flatMap((entry) => (entry.kind === 'prompt' ? [entry.text] : []));
  }

  /** The first prompt shown, to name the run by. */
  firstPrompt(): string | null {
    const prompt = this.entries.find((entry) => entry.kind === 'prompt');
    return prompt?.kind === 'prompt' ? prompt.text : null;
  }

  static #itemOf(entry: Entry): ItemId | null {
    return 'item' in entry ? entry.item : null;
  }

  static done(entry: Entry): boolean {
    switch (entry.kind) {
      case 'message':
      case 'reasoning':
      case 'tool':
        return entry.done;
      case 'command':
      case 'file_change':
      case 'questions':
        return entry.status !== 'in_progress';
      case 'prompt':
      case 'turn_end':
      case 'notice':
        return true;
      default:
        return assertNever(entry);
    }
  }

  /** Whether `one` and `two` are the same entry, said twice. */
  static same(one: Entry, two: Entry): boolean {
    const [first, second] = [Transcript.#itemOf(one), Transcript.#itemOf(two)];
    if (first !== null && second !== null) return first === second;
    if (one.kind === 'turn_end' && two.kind === 'turn_end') return one.turn === two.turn;
    return JSON.stringify(one) === JSON.stringify(two);
  }

  /**
   * The conversation after `event`, which process `run` of it reported. A
   * client names its items afresh in each process (Pi counts them from the
   * first), so an item is the same item only within its process; without
   * a process, by its name alone.
   */
  apply(event: DriveEvent, run: RunId | null = null): Transcript {
    const own = (item: ItemId): ItemId => (run === null ? item : (`${run}/${item}` as ItemId));
    switch (event.kind) {
      case 'turn_started':
        return new Transcript(this.entries, event.turn, this.read, this.earlier);
      case 'delta': {
        const item = own(event.item);
        const index = this.#find(item);
        const entry = index === -1 ? undefined : this.entries[index];
        if (entry?.kind === 'message') {
          return this.#replace(index, { ...entry, text: entry.text + event.text });
        }
        return this.#push({ kind: 'message', item, text: event.text, done: false });
      }
      case 'item':
        return this.#item({ ...event.item, id: own(event.item.id) }, event.phase === 'completed');
      case 'turn_ended':
        return new Transcript(this.entries, event.turn, this.read, this.earlier).#push({
          kind: 'turn_end',
          turn: event.turn,
          end: event.end,
        });
      case 'error':
        return this.#append({
          kind: 'notice',
          tone: event.retry === 'will_retry' ? 'warn' : 'danger',
          text: event.retry === 'will_retry' ? `${event.message} (retrying)` : event.message,
        });
      case 'refused':
        return this.#append({ kind: 'notice', tone: 'danger', text: event.message });
      case 'ready':
      case 'approval':
      case 'approval_resolved':
      case 'unsupported':
      case 'other':
        // Approvals live in the run's state; the rest says nothing to show.
        return this;
      default:
        return assertNever(event);
    }
  }

  #item(item: DriveItem, done: boolean): Transcript {
    const read = item.read;
    let entry: Entry | null;
    switch (read.as) {
      case 'prompt':
        entry =
          read.text === '' && read.images.length === 0
            ? null
            : { kind: 'prompt', item: item.id, text: read.text, images: read.images };
        break;
      case 'reply': {
        const index = this.#find(item.id);
        const before = index === -1 ? undefined : this.entries[index];
        const streamed = before?.kind === 'message' ? before.text : '';
        entry = { kind: 'message', item: item.id, text: read.text || streamed, done };
        break;
      }
      case 'reasoning':
        entry = { kind: 'reasoning', item: item.id, done };
        break;
      case 'command':
        entry = {
          kind: 'command',
          item: item.id,
          command: read.command,
          intent: read.intent,
          status: read.status,
          output: read.output,
          exit: read.exit_code,
        };
        break;
      case 'file_change':
        entry = { kind: 'file_change', item: item.id, paths: read.paths, status: read.status };
        break;
      case 'plan':
        entry = read.text ? { kind: 'message', item: item.id, text: read.text, done } : null;
        break;
      case 'tool':
        entry = { kind: 'tool', item: item.id, name: read.name, effects: read.effects, done };
        break;
      case 'questions':
        entry = {
          kind: 'questions',
          item: item.id,
          questions: read.questions,
          answers: read.answers,
          status: read.status,
        };
        break;
      case 'unread':
        entry = { kind: 'tool', item: item.id, name: item.kind, effects: [], done };
        break;
      default:
        return assertNever(read);
    }
    return entry === null ? this : this.#push(entry);
  }

  #find(item: ItemId): number {
    return this.entries.findLastIndex((entry) => Transcript.#itemOf(entry) === item);
  }

  #replace(index: number, entry: Entry): Transcript {
    const entries = this.entries.slice();
    entries[index] = entry;
    return new Transcript(entries, this.turn, this.read, this.earlier);
  }

  #append(entry: Entry): Transcript {
    return new Transcript([...this.entries, entry], this.turn, this.read, this.earlier);
  }

  /** Add `entry`, or put it in the place of the same entry said before. */
  #push(entry: Entry): Transcript {
    const at = this.entries.findLastIndex((shown) => Transcript.same(shown, entry));
    return at === -1 ? this.#append(entry) : this.#replace(at, entry);
  }
}

/**
 * Whether the running turn can be stopped: `none` when no turn runs,
 * `pending` while a turn is under way but its client has not yet named it
 * (the daemon refuses to interrupt until it does), `ready` once it has.
 */
export type TurnStop = 'none' | 'pending' | 'ready';

/** What the operator can ask of a run in its current state. */
export interface RunControls {
  prompt: boolean;
  interrupt: TurnStop;
  stop: boolean;
  /** The approvals waiting for an answer, oldest first. */
  approvals: Waiting[];
}

/** A session with no client running: a prompt starts one; nothing else is asked of it. */
export const UNSTARTED: RunControls = {
  prompt: true,
  interrupt: 'none',
  stop: false,
  approvals: [],
};

/** How a run's state reads in the sidebar and header: its mark, and the
 * words the mark's tooltip says. */
export interface PhaseMark {
  label: string;
  mark: Mark;
}

export class Runs {
  /** A conversation's turns: each starts at its prompt. */
  static turns(entries: readonly Entry[]): Entry[][] {
    const turns: Entry[][] = [];
    for (const entry of entries) {
      if (entry.kind === 'prompt' || turns.length === 0) turns.push([]);
      turns.at(-1)!.push(entry);
    }
    return turns;
  }

  static controls(state: RunState): RunControls {
    switch (state.phase) {
      case 'idle':
        return { prompt: true, interrupt: 'none', stop: true, approvals: [] };
      case 'working':
        return {
          prompt: false,
          interrupt: state.turn === null ? 'pending' : 'ready',
          stop: true,
          approvals: state.approvals,
        };
      case 'starting':
        return { prompt: false, interrupt: 'none', stop: true, approvals: [] };
      case 'stopping':
      case 'ended':
      case 'unrecognized':
        return { prompt: false, interrupt: 'none', stop: false, approvals: [] };
      default:
        return assertNever(state);
    }
  }

  /** A run's state as a mark, with its words. Only an approval it waits on
   * is the operator's: a run between turns rests. */
  static mark(state: RunState): PhaseMark {
    switch (state.phase) {
      case 'starting':
        return { label: 'Starting', mark: 'waiting' };
      case 'idle':
        return { label: 'Your turn', mark: 'idle' };
      case 'working':
        return state.approvals.length
          ? { label: 'Needs you', mark: 'yours' }
          : { label: 'Working', mark: 'responding' };
      case 'stopping':
        return { label: 'Stopping', mark: 'waiting' };
      case 'ended':
        return {
          label: Runs.endLabel(state.end, state.exit),
          mark: Runs.endMark(state.end, state.exit),
        };
      case 'unrecognized':
        return { label: state.text, mark: 'unknown' };
      default:
        return assertNever(state);
    }
  }

  /** How a run ended, as the operator would say it; the process's exit is a
   * detail ({@link Runs.exitDetail}), not the headline. */
  static endLabel(end: Stored<RunEnd>, exit: ProcessExit | null = null): string {
    switch (end) {
      case 'spawn_failed':
        return 'Couldn’t start';
      case 'open_refused':
        return 'Couldn’t open its conversation';
      case 'exited':
        switch (exit?.kind) {
          case 'code':
            return exit.value === 0 ? 'Ended' : 'Ended with an error';
          case 'signal':
            return 'Killed';
          case 'unknown':
          case undefined:
            return 'Ended';
          default:
            return assertNever(exit);
        }
      case 'stopped':
        return 'Stopped';
      case 'drained':
        return 'Stopped when Ren shut down';
      case 'recovered':
        return 'Lost when Ren restarted';
      case 'idled':
        return 'Resting';
      default:
        return unrecognised(end);
    }
  }

  /** How a run's ending marks it: a clean stop rests, an ending that lost
   * work needs a look, and a client that never ran failed. */
  static endMark(end: Stored<RunEnd>, exit: ProcessExit | null = null): Mark {
    switch (end) {
      case 'stopped':
      case 'idled':
        return 'idle';
      case 'exited':
        return exit?.kind === 'code' && exit.value === 0 ? 'idle' : 'warning';
      case 'drained':
      case 'recovered':
        return 'warning';
      case 'spawn_failed':
      case 'open_refused':
        return 'failed';
      default:
        unrecognised(end);
        return 'unknown';
    }
  }

  /** How the process exited, for the run's details: `exit code 1`, or the
   * signal by name, `killed by SIGKILL (signal 9)`. */
  static exitDetail(exit: ProcessExit | null): string | null {
    const signals: Record<number, string> = {
      1: 'SIGHUP',
      2: 'SIGINT',
      6: 'SIGABRT',
      9: 'SIGKILL',
      11: 'SIGSEGV',
      15: 'SIGTERM',
    };
    switch (exit?.kind) {
      case 'code':
        return `exit code ${exit.value}`;
      case 'signal': {
        const name = signals[exit.value];
        return name ? `killed by ${name} (signal ${exit.value})` : `killed by signal ${exit.value}`;
      }
      case 'unknown':
      case undefined:
        return null;
      default:
        return assertNever(exit);
    }
  }

  /**
   * Where a run works, as the operator would say it: Scratch for a scratch
   * run, else its folder as `~/…` where a place offered shows the home.
   */
  static place(workdir: string, scratch: string | null, offered: readonly RunPlace[]): string {
    if (scratch && (workdir === scratch || workdir.startsWith(`${scratch}/`))) return 'Scratch';
    const known = offered.find((place) => place.path === workdir);
    if (known) return known.shown;
    // The home, as a place offered under it shows it.
    const homed = offered.find((place) => place.shown.startsWith('~/'));
    const home = homed ? homed.path.slice(0, homed.path.length - homed.shown.length + 1) : null;
    return home && workdir.startsWith(`${home}/`) ? `~${workdir.slice(home.length)}` : workdir;
  }

  /** One line saying what an approval asks. */
  static ask(ask: ApprovalAsk): { action: string; detail: string | null; reason: string | null } {
    switch (ask.kind) {
      case 'command':
        return {
          action: 'Run a command',
          detail: ask.command,
          reason: ask.reason,
        };
      case 'file_change':
        return { action: 'Change files', detail: null, reason: ask.reason };
      case 'tool':
        return { action: `Use ${ask.tool}`, detail: ask.detail, reason: null };
      case 'questions':
        return {
          action: ask.title ?? 'Answer a question',
          detail: Questions.summary(ask.questions),
          reason: null,
        };
      case 'unrecognized':
        return { action: `Allow ${ask.text}`, detail: null, reason: null };
      default:
        return assertNever(ask);
    }
  }

  /** Why a run cannot start with a client, or null when it can. */
  static unavailable(status: Stored<ClientStatus>): string | null {
    switch (status) {
      case 'ready':
        return null;
      case 'not_installed':
        return 'not installed on the daemon’s host';
      default:
        return unrecognised(status);
    }
  }

  /**
   * Projects grouped by the folder they are in, groups in path order, and
   * each group's projects by name.
   */
  /** Places by the folder they are in, as the operator reads it (`~/dev`). */
  static places(projects: readonly RunPlace[]): { parent: string; projects: RunPlace[] }[] {
    const groups = new Map<string, RunPlace[]>();
    for (const project of projects) {
      const cut = project.shown.lastIndexOf('/');
      const parent = cut > 0 ? project.shown.slice(0, cut) : project.shown;
      groups.set(parent, [...(groups.get(parent) ?? []), project]);
    }
    return [...groups]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([parent, projects]) => ({
        parent,
        projects: projects.toSorted((a, b) => a.name.localeCompare(b.name)),
      }));
  }

  /** The projects matching what is typed, by name or path, ignoring case. */
  static choose(projects: readonly RunPlace[], typed: string): RunPlace[] {
    const text = typed.trim().toLowerCase();
    return projects.filter(
      (project) =>
        project.name.toLowerCase().includes(text) || project.shown.toLowerCase().includes(text),
    );
  }

  /** What a project's repository is on, in a few words. */
  /** Whether a folder is a git repository whose `HEAD` was read. */
  static versioned(git: PlaceGit): boolean {
    return git.kind === 'branch' || git.kind === 'detached';
  }

  static branch(git: PlaceGit): string {
    switch (git.kind) {
      case 'branch':
        return git.name;
      case 'detached':
        return `at ${git.commit}`;
      case 'unreadable':
        return 'git';
      case 'none':
        return 'no git';
      default:
        return assertNever(git);
    }
  }

  /** A run's client, when the daemon can drive it to resume the conversation. */
  static drivable(harness: Stored<Harness>): Drivable | null {
    return DRIVABLE.find((drivable) => drivable === harness) ?? null;
  }

  /** The folder's last component, as the sidebar names a project. */
  /** `path` as read from inside `workdir`: relative there, else as it is. */
  static within(path: string, workdir: string | null): string {
    if (!workdir) return path;
    const root = workdir.endsWith('/') ? workdir : `${workdir}/`;
    return path.startsWith(root) && path.length > root.length ? path.slice(root.length) : path;
  }

  static folder(workdir: string): string {
    const parts = workdir.split('/').filter((part) => part !== '');
    return parts.at(-1) ?? workdir;
  }

  /** The thread a run's conversation has, while the run has one open. */
  static thread(state: RunState): NativeThreadId | null {
    switch (state.phase) {
      case 'idle':
      case 'working':
        return state.thread;
      case 'starting':
      case 'stopping':
      case 'ended':
      case 'unrecognized':
        return null;
      default:
        return assertNever(state);
    }
  }

  /** Apply an update to the live list: a new run appears, a known one changes. */
  static update(runs: RunSnapshot[], update: RunUpdate): RunSnapshot[] {
    if (update.kind !== 'state') return runs;
    const index = runs.findIndex((run) => run.run === update.run);
    if (index === -1) {
      // A run started elsewhere; its folder arrives with the next listing.
      return runs;
    }
    const next = runs.slice();
    next[index] = { ...runs[index]!, state: update.state };
    return next;
  }
}

/** A run in the sidebar: live, or recorded and ended. */
export interface DeskRun {
  run: RunId;
  harness: Stored<Harness>;
  workdir: string;
  started_at: UnixMillis | null;
  /** The live state, while the daemon holds the run. */
  state: RunState | null;
  recorded: RecordedRun | null;
  /** What it was started with: as the daemon holds it, or as recorded. */
  settings: RunSettings;
}

/** Runs grouped by folder, the folder with the newest run first. */
export interface Project {
  workdir: string;
  name: string;
  runs: DeskRun[];
  /** Approvals waiting across the folder's runs. */
  waiting: number;
}

export class Desk {
  /**
   * Join live and recorded runs and group them by folder. Ended runs in
   * `hidden` (archived) are left out, though the daemon still lists them.
   */
  static projects(
    live: RunSnapshot[],
    recorded: RecordedRun[],
    hidden: ReadonlySet<RunId> = new Set(),
    scratch: string | null = null,
  ): Project[] {
    const byRun = new Map<RunId, DeskRun>();
    for (const run of recorded) {
      byRun.set(run.id, {
        run: run.id,
        harness: run.harness,
        workdir: run.workdir,
        started_at: run.started_at,
        state: null,
        recorded: run,
        settings: { model: run.model, effort: run.effort, approvals: run.approvals },
      });
    }
    for (const run of live) {
      if (run.state.phase === 'ended' && hidden.has(run.run)) continue;
      const known = byRun.get(run.run);
      byRun.set(run.run, {
        run: run.run,
        harness: run.harness,
        workdir: run.workdir,
        started_at: known?.started_at ?? null,
        state: run.state,
        recorded: known?.recorded ?? null,
        settings: run.settings,
      });
    }
    // Scratch runs each have a folder of their own: they share one group.
    const group = (workdir: string) =>
      scratch && workdir.startsWith(`${scratch}/`) ? scratch : workdir;
    const groups = new Map<string, DeskRun[]>();
    for (const run of byRun.values()) {
      const runs = groups.get(group(run.workdir)) ?? [];
      runs.push(run);
      groups.set(group(run.workdir), runs);
    }
    // Live runs first, then newest first; a run not yet recorded is newest.
    const newest = (run: DeskRun) => run.started_at ?? Number.MAX_SAFE_INTEGER;
    const live_ = (run: DeskRun) => (run.state !== null && run.state.phase !== 'ended' ? 1 : 0);
    const projects = [...groups].map(([workdir, runs]) => {
      runs.sort((a, b) => live_(b) - live_(a) || newest(b) - newest(a));
      return {
        workdir,
        name: workdir === scratch ? 'Scratch' : Runs.folder(workdir),
        runs,
        waiting: runs.reduce(
          (sum, run) => sum + (run.state ? Runs.controls(run.state).approvals.length : 0),
          0,
        ),
      };
    });
    projects.sort(
      (a, b) =>
        b.waiting - a.waiting ||
        Math.max(...b.runs.map(live_)) - Math.max(...a.runs.map(live_)) ||
        newest(b.runs[0]!) - newest(a.runs[0]!),
    );
    return projects;
  }

  /** The projects runs worked in, most recently started first, at most `count`. */
  static recent(
    runs: readonly { workdir: string; started_at: UnixMillis | null }[],
    offered: readonly RunPlace[],
    count = 3,
  ): RunPlace[] {
    const newest = runs
      .filter((run) => run.started_at !== null)
      .toSorted((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0));
    const recent: RunPlace[] = [];
    for (const run of newest) {
      const project = offered.find((project) => project.path === run.workdir);
      if (project && !recent.includes(project)) recent.push(project);
      if (recent.length === count) break;
    }
    return recent;
  }

  /** A folder's runs that have not ended, then those that have, newest first. */
  static split(project: Project): { open: DeskRun[]; ended: DeskRun[] } {
    const ended = (run: DeskRun) => Desk.state(run).phase === 'ended';
    return {
      open: project.runs.filter((run) => !ended(run)),
      ended: project.runs.filter(ended),
    };
  }

  /** The state a sidebar run shows: live if held, else its recorded end. */
  static state(run: DeskRun): RunState {
    if (run.state) return run.state;
    const recorded = run.recorded;
    if (recorded?.end) return { phase: 'ended', end: recorded.end, exit: recorded.exit };
    // Recorded as started but not held by the daemon: it ended without a record yet.
    return { phase: 'unrecognized', text: 'not running' };
  }
}

/** A session in the sidebar, with the state of its newest process. */
export interface DeskSession {
  session: SessionId;
  harness: Stored<Harness>;
  workdir: string;
  /** When it was last active: its newest run's start, else its creation; null until stored. */
  activeAt: number | null;
  phase: Stored<SessionPhase>;
  shelf: Stored<'open' | 'archived'>;
  /** What it was first asked, as stored; null until a prompt is recorded. */
  asked: string | null;
  /** What its next turn runs on. */
  settings: RunSettings;
  /** Its newest process, once one started. */
  run: RunId | null;
  /** That process's state: live while the daemon holds it, else its recorded end. */
  state: RunState | null;
  /** Its runs as recorded, newest first. */
  runs: RecordedRun[];
  /** Whether its client takes images; null until the daemon serves it. */
  images: Stored<ImageTaking> | null;
  /** The worktree it works in; null in any other folder. */
  worktree: Worktree | null;
}

/** Sessions grouped by folder, folders that need you first. */
export interface SessionFolder {
  workdir: string;
  name: string;
  sessions: DeskSession[];
  /** Approvals waiting across the folder's sessions. */
  waiting: number;
}

/** A project as a session desk knows it: enough to tell its sessions. */
export type DeskProject = Pick<RegisteredProject, 'id' | 'name' | 'repository'>;

/** Which sessions a desk holds: one project's, or those working for none
 * (Scratch: scratch folders, and folders no project is registered for). */
export type SessionScope =
  { kind: 'project'; project: DeskProject } | { kind: 'scratch'; projects: readonly DeskProject[] };

export class Sessions {
  /**
   * The project among `projects` that `body` works for: as stored, else
   * (for a moment after it is made) by its folder, the project's
   * repository or a worktree of it. Null when it works for none of them.
   */
  static projectOf(body: SessionBody, projects: readonly DeskProject[]): DeskProject | null {
    if (body.stored) {
      const id = body.stored.project;
      return id === null ? null : (projects.find((project) => project.id === id) ?? null);
    }
    const workdir = body.served?.workdir;
    return (
      projects.find(
        (project) =>
          workdir === project.repository ||
          (body.worktree !== null &&
            body.worktree.name.replace(/-\d+$/, '') === Sessions.#base(project.repository)),
      ) ?? null
    );
  }

  /** The sessions among `bodies` that `scope` holds. */
  static within(bodies: readonly SessionBody[], scope: SessionScope): SessionBody[] {
    switch (scope.kind) {
      case 'project':
        return bodies.filter((body) => Sessions.projectOf(body, [scope.project]) !== null);
      case 'scratch':
        return bodies.filter((body) => Sessions.projectOf(body, scope.projects) === null);
      default:
        return assertNever(scope);
    }
  }

  /** The desk a session is on: its project's, by name, or Scratch (null). */
  static deskHref(project: string | null, query: Record<string, string> = {}): string {
    const search = new URLSearchParams(query).toString();
    const path =
      project === null ? '/scratch' : `/projects/${encodeURIComponent(project)}/sessions`;
    return search ? `${path}?${search}` : path;
  }

  static #base(path: string): string {
    return path.replace(/\/+$/, '').split('/').pop() ?? '';
  }

  /** One session from what the store holds and what the daemon serves. */
  static desk(body: SessionBody, live: readonly RunSnapshot[]): DeskSession | null {
    const { stored, served } = body;
    const first = served ?? stored;
    if (!first) return null;
    const run = served?.process ?? stored?.runs[0]?.id ?? null;
    const held = run ? live.find((entry) => entry.run === run) : undefined;
    const recordedRun = run ? stored?.runs.find((entry) => entry.id === run) : undefined;
    const state: RunState | null = held
      ? held.state
      : recordedRun?.end
        ? { phase: 'ended', end: recordedRun.end, exit: recordedRun.exit }
        : run
          ? { phase: 'unrecognized', text: 'not running' }
          : null;
    return {
      session: body.session,
      harness: first.harness,
      workdir: first.workdir,
      activeAt: stored ? (stored.runs[0]?.started_at ?? stored.created_at) : null,
      phase: served?.state.phase ?? (stored?.first_turn_at ? 'active' : 'draft'),
      shelf: served?.state.shelf ?? (stored?.archived_at ? 'archived' : 'open'),
      asked: stored?.first_prompt ?? null,
      settings: served?.settings ?? {
        model: stored?.model ?? null,
        effort: stored?.effort ?? null,
        approvals: stored?.approvals ?? null,
      },
      run,
      state,
      runs: stored?.runs ?? [],
      images: served?.images ?? null,
      worktree: body.worktree,
    };
  }

  /** Whether the session's process is running now. */
  static running(session: DeskSession): boolean {
    return session.state !== null && !['ended', 'unrecognized'].includes(session.state.phase);
  }

  /** Whether the session is a current draft never asked anything: not
   * listed unless it is open, and discarded once its client sits idle. */
  static untouched(session: DeskSession): boolean {
    return session.phase === 'draft' && session.shelf === 'open' && session.asked === null;
  }

  /**
   * The sessions grouped by folder: scratch sessions share one group.
   * Folders with an approval waiting come first, then those with a process
   * running, then the most recently active; the same within a folder. A
   * draft never asked anything is left out unless it is `open`.
   */
  static folders(
    bodies: readonly SessionBody[],
    live: readonly RunSnapshot[],
    scratch: string | null = null,
    open: SessionId | null = null,
  ): SessionFolder[] {
    const group = (workdir: string) =>
      scratch && workdir.startsWith(`${scratch}/`) ? scratch : workdir;
    const groups = new Map<string, DeskSession[]>();
    for (const body of bodies) {
      const session = Sessions.desk(body, live);
      if (!session) continue;
      if (Sessions.untouched(session) && session.session !== open) continue;
      const sessions = groups.get(group(session.workdir)) ?? [];
      sessions.push(session);
      groups.set(group(session.workdir), sessions);
    }
    const waiting = (session: DeskSession) =>
      session.state ? Runs.controls(session.state).approvals.length : 0;
    // A session not stored yet was just made: it is the newest.
    const newest = (session: DeskSession) => session.activeAt ?? Number.MAX_SAFE_INTEGER;
    const running = (session: DeskSession) => (Sessions.running(session) ? 1 : 0);
    const order = (a: DeskSession, b: DeskSession) =>
      waiting(b) - waiting(a) || running(b) - running(a) || newest(b) - newest(a);
    const folders = [...groups].map(([workdir, sessions]) => ({
      workdir,
      name: workdir === scratch ? 'Scratch' : Runs.folder(workdir),
      sessions: sessions.toSorted(order),
      waiting: sessions.reduce((sum, session) => sum + waiting(session), 0),
    }));
    return folders.toSorted((a, b) => order(a.sessions[0]!, b.sessions[0]!));
  }

  /**
   * How a session stands in a word: its process's state while one runs (a
   * draft's idle one says Draft); with none running, a client that could
   * not start says so, a draft says Draft, and any other names its client
   * quietly, as writing to it starts one again.
   */
  static mark(session: DeskSession, client: string): PhaseMark {
    const state = session.state;
    if (state && Sessions.running(session)) {
      return state.phase === 'idle' && session.phase === 'draft'
        ? { label: 'Draft', mark: 'draft' }
        : Runs.mark(state);
    }
    if (state?.phase === 'ended' && Runs.endMark(state.end, state.exit) === 'failed') {
      return Runs.mark(state);
    }
    return session.phase === 'draft'
      ? { label: 'Draft', mark: 'draft' }
      : { label: client, mark: 'idle' };
  }

  /**
   * The newest draft never asked anything, in any folder, for a new session
   * to reuse instead of making another (a draft's folder can change); null
   * when there is none.
   */
  static unused(bodies: readonly SessionBody[], live: readonly RunSnapshot[]): DeskSession | null {
    const drafts = bodies
      .map((body) => Sessions.desk(body, live))
      .filter((session): session is DeskSession => session !== null && Sessions.untouched(session));
    const newest = (session: DeskSession) => session.activeAt ?? Number.MAX_SAFE_INTEGER;
    return drafts.toSorted((a, b) => newest(b) - newest(a))[0] ?? null;
  }

  /**
   * Where a new session starts: the folder of the session last active that
   * was asked something, as a run root's or another place's folder; null
   * for a scratch folder of its own, as the last one had or when there is
   * none. `undefined` when neither is possible: no session to follow and no
   * scratch folder.
   */
  static start(
    bodies: readonly SessionBody[],
    live: readonly RunSnapshot[],
    scratch: string | null,
  ): string | null | undefined {
    const asked = bodies
      .map((body) => Sessions.desk(body, live))
      .filter((session): session is DeskSession => session !== null && !Sessions.untouched(session))
      .toSorted((a, b) => (b.activeAt ?? 0) - (a.activeAt ?? 0))[0];
    if (asked && !(scratch && Sessions.inScratch(asked.workdir, scratch))) return asked.workdir;
    return scratch === null && !asked ? undefined : null;
  }

  /** Whether `workdir` is a scratch folder of its own, under `scratch`. */
  static inScratch(workdir: string, scratch: string | null): boolean {
    return scratch !== null && workdir.startsWith(`${scratch}/`);
  }

  /** The session one of whose runs is `run`, for links to a run. */
  static of(bodies: readonly SessionBody[], run: RunId): SessionId | null {
    return (
      bodies.find(
        (body) =>
          body.served?.process === run || body.stored?.runs.some((entry) => entry.id === run),
      )?.session ?? null
    );
  }
}

/** A client's models as the new-run form offers them. */
export class Models {
  /** The models by provider, in the order the client lists them. */
  static groups(models: readonly RunModel[]): { provider: string | null; models: RunModel[] }[] {
    const groups: { provider: string | null; models: RunModel[] }[] = [];
    for (const model of models) {
      const group = groups.find((group) => group.provider === model.provider);
      if (group) group.models.push(model);
      else groups.push({ provider: model.provider, models: [model] });
    }
    return groups;
  }

  /** The model a run gets when none is chosen, when the client says. */
  static fallback(models: readonly RunModel[]): RunModel | null {
    return models.find((model) => model.choice === 'default') ?? null;
  }

  /** `wanted` when the client still lists it; otherwise no choice. */
  static keep(models: readonly RunModel[], wanted: string | null): ModelId | null {
    return models.find((model) => model.id === wanted)?.id ?? null;
  }
}

/** A run's settings as people read them. */
export class Setup {
  /** An approval policy in a few words. */
  static policy(policy: Stored<ApprovalPolicy>): string {
    switch (policy) {
      case 'ask':
        return 'Ask first';
      case 'edits':
        return 'Edits alone';
      case 'auto':
        return 'Auto';
      default:
        return policy;
    }
  }

  /** What each policy lets the client do, in a sentence. */
  static explain(policy: ApprovalPolicy): string {
    switch (policy) {
      case 'ask':
        return 'Every edit and command waits for you.';
      case 'edits':
        return 'Edits in the folder go ahead; commands wait for you.';
      case 'auto':
        return 'The client decides what is safe and asks only beyond that.';
      default:
        return assertNever(policy);
    }
  }

  /** The settings that are not the client's own, as short words. */
  static words(settings: RunSettings): string[] {
    return [
      settings.model,
      settings.effort ? `${settings.effort} effort` : null,
      settings.approvals ? Setup.policy(settings.approvals).toLowerCase() : null,
    ].filter((word): word is string => word !== null);
  }

  /** The settings to start again with: labels this UI does not know are
   * left to the client. */
  static known(settings: RunSettings): ChosenSettings {
    return {
      model: settings.model,
      effort: EFFORTS.find((level) => level === settings.effort) ?? null,
      approvals: APPROVAL_POLICIES.find((policy) => policy === settings.approvals) ?? null,
    };
  }

  /** The option that stands for the client's own choice, named for what it
   * is: "Opus 5.5 (default)". */
  static own(known: string): string {
    return `${known} (default)`;
  }

  /** The efforts the chosen model takes: the default's are its model's. */
  static efforts(models: readonly RunModel[], model: ModelId | null): RunModel | null {
    return (
      (model ? models.find((offered) => offered.id === model) : Models.fallback(models)) ?? null
    );
  }
}

/** The settings last chosen for a client, as kept for the viewer. */
export interface Kept {
  model?: string;
  effort?: string;
  approvals?: string;
}

/** The client last chosen for a session, and each client's last settings. */
export interface Remembered {
  harness: Drivable;
  settings: Partial<Record<Drivable, Kept>>;
}

/**
 * What the Sessions page remembers for this viewer, so a new session starts
 * as the last one was set: kept in a cookie. Only labels: never a prompt, a
 * path, or anything secret. Read as untrusted text.
 */
export class Memory {
  static readonly COOKIE = 'ren_runs';
  /** A viewer with nothing remembered starts on Claude Code. */
  static readonly NONE: Remembered = { harness: 'claude_code', settings: {} };

  /** The cookie's value, read; anything unreadable is the default. */
  static parse(raw: string | null | undefined): Remembered {
    if (!raw) return Memory.NONE;
    let saved: unknown;
    try {
      saved = JSON.parse(raw);
    } catch {
      return Memory.NONE;
    }
    if (!isRecord(saved)) return Memory.NONE;
    const settings: Partial<Record<Drivable, Kept>> = {};
    const stored = saved.settings;
    if (isRecord(stored)) {
      for (const drivable of DRIVABLE) {
        const kept = stored[drivable];
        if (!isRecord(kept)) continue;
        const text = (key: string) => {
          const value = kept[key];
          return typeof value === 'string' && value.length <= 200 ? value : undefined;
        };
        settings[drivable] = {
          model: text('model'),
          effort: text('effort'),
          approvals: text('approvals'),
        };
      }
    }
    return {
      harness: DRIVABLE.find((drivable) => drivable === saved.harness) ?? Memory.NONE.harness,
      settings,
    };
  }

  /** The cookie that keeps `remembered` for a year, for the Sessions page only. */
  static cookie(remembered: Remembered): string {
    const value = encodeURIComponent(JSON.stringify(remembered));
    return `${Memory.COOKIE}=${value}; Path=/sessions; Max-Age=31536000; SameSite=Lax`;
  }

  /** `remembered`, with `harness` the client last chosen. */
  static chose(remembered: Remembered, harness: Drivable): Remembered {
    return { ...remembered, harness };
  }

  /** `remembered`, with `settings` the last chosen for `harness`; none is the client's own. */
  static set(remembered: Remembered, harness: Drivable, settings: ChosenSettings): Remembered {
    return {
      ...remembered,
      settings: {
        ...remembered.settings,
        [harness]: {
          model: settings.model ?? undefined,
          effort: settings.effort ?? undefined,
          approvals: settings.approvals ?? undefined,
        },
      },
    };
  }

  /** The settings a new session on `harness` starts with: those kept for it
   * that still apply, the model one it lists, the effort one that model
   * takes, and the approval policy one it runs under. */
  static settings(
    remembered: Remembered,
    harness: Drivable,
    models: readonly RunModel[],
    policies: readonly Stored<ApprovalPolicy>[],
  ): { model: ModelId | null; effort: Effort | null; approvals: ApprovalPolicy | null } {
    const kept = remembered.settings[harness] ?? {};
    const model = Models.keep(models, kept.model ?? null);
    const takes = Setup.efforts(models, model)?.efforts ?? [];
    return {
      model,
      effort: EFFORTS.find((level) => level === kept.effort && takes.includes(level)) ?? null,
      approvals:
        APPROVAL_POLICIES.find(
          (policy) => policy === kept.approvals && policies.includes(policy),
        ) ?? null,
    };
  }
}

/** What the composer of a run does, by how the run stands. */
export type ComposeMode =
  /** No turn runs: Enter starts one. */
  | { kind: 'start' }
  /** A turn runs: Enter queues for after it; "send now" steers it where the client can. */
  | { kind: 'during'; steer: boolean; queued: string | null }
  /** The run ended with a conversation: writing continues it in a new run. */
  | { kind: 'continue' }
  /** Nothing can be sent yet, or any more: why. */
  | { kind: 'closed'; reason: string };

export class Compose {
  /** How the composer of a run in `state` works, for a client taking `midway`. */
  static mode(state: RunState, midway: readonly Stored<Midway>[], resumable: boolean): ComposeMode {
    switch (state.phase) {
      case 'idle':
        return { kind: 'start' };
      case 'working':
        return { kind: 'during', steer: midway.includes('steer'), queued: state.queued };
      case 'starting':
        return { kind: 'closed', reason: 'The run is opening its conversation…' };
      case 'stopping':
        return { kind: 'closed', reason: 'The run is stopping.' };
      case 'ended':
        return resumable
          ? { kind: 'continue' }
          : { kind: 'closed', reason: 'This run ended before it opened a conversation.' };
      case 'unrecognized':
        return { kind: 'closed', reason: 'The daemon reports a state this page does not know.' };
      default:
        return assertNever(state);
    }
  }

  /**
   * The earlier prompt ↑ or ↓ recalls: `prompts` oldest first, `at` how
   * far back the composer is now (null: not recalling). Null when there is
   * nothing further that way, `at` null again when back past the newest.
   */
  /**
   * How a session's composer works: a prompt always goes, starting or
   * resuming its process when none runs, or into or after a running turn;
   * only while its process stops does it wait.
   */
  static session(state: RunState | null, midway: readonly Stored<Midway>[]): ComposeMode {
    if (!state) return { kind: 'start' };
    switch (state.phase) {
      case 'working':
        return { kind: 'during', steer: midway.includes('steer'), queued: state.queued };
      case 'stopping':
        return { kind: 'closed', reason: 'Its client is stopping; write again in a moment.' };
      case 'idle':
      case 'starting':
      case 'ended':
      case 'unrecognized':
        return { kind: 'start' };
      default:
        return assertNever(state);
    }
  }

  static recall(
    prompts: readonly string[],
    at: number | null,
    older: boolean,
  ): { at: number | null; text: string } | null {
    if (older) {
      const next = at === null ? 0 : at + 1;
      const text = prompts[prompts.length - 1 - next];
      return text === undefined ? null : { at: next, text };
    }
    if (at === null) return null;
    if (at === 0) return { at: null, text: '' };
    return { at: at - 1, text: prompts[prompts.length - at]! };
  }
}

/** Where a session's delete was asked: its row in the list, or its header. */
export type DeleteOrigin = 'list' | 'header';

/**
 * A session's delete, asked and confirmed before it is sent: one at a time.
 * Done is not a state of its own: the session leaves the lists, and the
 * delete with it.
 */
export type Deletion =
  | { phase: 'idle' }
  | { phase: 'confirming'; session: SessionId; from: DeleteOrigin }
  | { phase: 'deleting'; session: SessionId; from: DeleteOrigin }
  | { phase: 'failed'; session: SessionId; from: DeleteOrigin; reason: string };

/** What happens to a delete. */
export type DeletionEvent =
  | { kind: 'ask'; session: SessionId; from: DeleteOrigin }
  | { kind: 'cancel' }
  | { kind: 'confirm' }
  | { kind: 'refused'; reason: string }
  | { kind: 'gone' };

/** A delete event that does not apply in the state it came in. */
export interface IllegalDeletion {
  illegal: true;
  state: Deletion['phase'];
  event: DeletionEvent['kind'];
}

export class Deletions {
  static readonly idle: Deletion = { phase: 'idle' };

  /**
   * The delete after `event`. Asking again moves the question to another
   * session, but never while one is being deleted; a refused delete can be
   * confirmed again; cancelling while it is sent does not unsend it.
   */
  static step(state: Deletion, event: DeletionEvent): Deletion | IllegalDeletion {
    const illegal = (): IllegalDeletion => ({
      illegal: true,
      state: state.phase,
      event: event.kind,
    });
    switch (event.kind) {
      case 'ask':
        return state.phase === 'deleting'
          ? illegal()
          : { phase: 'confirming', session: event.session, from: event.from };
      case 'cancel':
        return state.phase === 'confirming' || state.phase === 'failed'
          ? Deletions.idle
          : illegal();
      case 'confirm':
        return state.phase === 'confirming' || state.phase === 'failed'
          ? { phase: 'deleting', session: state.session, from: state.from }
          : illegal();
      case 'refused':
        return state.phase === 'deleting'
          ? { phase: 'failed', session: state.session, from: state.from, reason: event.reason }
          : illegal();
      case 'gone':
        return state.phase === 'idle' ? illegal() : Deletions.idle;
      default:
        return assertNever(event);
    }
  }

  /** Whether `step` refused the event. */
  static illegal(next: Deletion | IllegalDeletion): next is IllegalDeletion {
    return 'illegal' in next;
  }

  /** The delete of `session` asked from `from`, when there is one. */
  static of(
    state: Deletion,
    session: SessionId,
    from: DeleteOrigin,
  ): Exclude<Deletion, { phase: 'idle' }> | null {
    return state.phase !== 'idle' && state.session === session && state.from === from
      ? state
      : null;
  }
}
