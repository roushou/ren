// A client's sign-in on the daemon's host, as the control API reports it:
// what the client's own status command last said, and one run of its own
// login, whose link the daemon relays and whose code it hands back. Parsed
// here at the boundary; the wording the Sessions view shows is pure and
// tested in tests/signIn.test.ts.
import { versioned } from './api.ts';
import { processExit, type ProcessExit } from './exit.ts';
import {
  assertNever,
  isRecord,
  label,
  nullable,
  object,
  string,
  unrecognised,
  ShapeError,
  type Decoder,
  type Stored,
  absentAs,
  tagValue,
  tagged,
  type Variant,
} from './decode.ts';
import type { LoginBody as WireLoginBody } from './wire/LoginBody.ts';
import type { LoginOutcome as WireLoginOutcome } from './wire/LoginOutcome.ts';
import type { OutcomeBody as WireOutcomeBody } from './wire/OutcomeBody.ts';
import type { SignInBody as WireSignInBody } from './wire/SignInBody.ts';
import type { WaitedBody as WireWaitedBody } from './wire/WaitedBody.ts';
import type { SignInState as WireSignInState } from './wire/SignInState.ts';
import type { LoginEnd as WireLoginEnd } from './wire/LoginEnd.ts';
import type { Login as WireLogin } from './wire/Login.ts';
import type { Waited as WireWaited } from './wire/Waited.ts';

export const SIGN_IN_METHODS = [
  'claude_subscription',
  'anthropic_console',
  'chatgpt',
  'anthropic_api_key',
  'openai_api_key',
] as const;
/** Which account a client signs in with. */
export type SignInMethod = (typeof SIGN_IN_METHODS)[number];
export const signInMethod = label(SIGN_IN_METHODS);

/** What the daemon knows of a client's sign-in. */
export type SignInState =
  | { kind: 'not_checked' }
  | { kind: 'checking' }
  /** `method` is the client's own word for how, shown as it is. */
  | { kind: 'signed_in'; method: string }
  | { kind: 'signed_out' }
  | { kind: 'unknown'; reason: string }
  /** A state a newer daemon reports, kept as its label. */
  | { kind: 'unrecognized'; label: string };

/** The page a login asks the operator to open: always `https`. */
export type SignInLink = string & { readonly __brand: 'SignInLink' };

/** How a login ended. Whether it signed the client in is the check after it. */
export type LoginEnd =
  | { end: 'exited'; exit: ProcessExit; said: string | null }
  | { end: 'cancelled' }
  | { end: 'timed_out' }
  | { end: 'broken'; said: string | null }
  | { end: 'unrecognized'; label: string };

/** One run of a client's login. */
export type Login =
  | { kind: 'starting'; said: string | null }
  | { kind: 'awaiting_code'; link: SignInLink; said: string | null }
  /** A device login: enter `code` on the page at `link`, and approve there. */
  | { kind: 'awaiting_approval'; link: SignInLink; code: string | null; said: string | null }
  /** Asking for an API key. */
  | { kind: 'awaiting_key'; said: string | null }
  /** Waiting for the address the browser could not open after approving at `link`. */
  | { kind: 'awaiting_redirect'; link: SignInLink; said: string | null }
  | { kind: 'finishing'; link: SignInLink | null; said: string | null }
  | { kind: 'ended'; end: LoginEnd };

/** Where a waited-on login stands: still going, or ended and checked. */
export type Waited = { kind: 'going'; login: Login } | { kind: 'done'; outcome: LoginOutcome };

/** A login's end, and the client's sign-in as checked after it. */
export interface LoginOutcome {
  end: LoginEnd;
  state: SignInState;
}

type StateOf<K> = Extract<SignInState, { kind: K }>;
type SentState<K> = Variant<WireSignInState, 'kind', K>;
export const signInState = tagged<SignInState, WireSignInState, 'kind'>(
  'kind',
  {
    not_checked: object<StateOf<'not_checked'>, SentState<'not_checked'>>({
      kind: tagValue('not_checked'),
    }),
    checking: object<StateOf<'checking'>, SentState<'checking'>>({ kind: tagValue('checking') }),
    signed_in: object<StateOf<'signed_in'>, SentState<'signed_in'>>({
      kind: tagValue('signed_in'),
      method: string,
    }),
    signed_out: object<StateOf<'signed_out'>, SentState<'signed_out'>>({
      kind: tagValue('signed_out'),
    }),
    unknown: object<StateOf<'unknown'>, SentState<'unknown'>>({
      kind: tagValue('unknown'),
      reason: string,
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { kind: 'unrecognized', label: string(value.kind, `${path}.kind`) };
  },
);

const signInLink: Decoder<SignInLink, string> = (value, path) => {
  const text = string(value, path);
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    throw new ShapeError(path, 'an https link');
  }
  if (url.protocol !== 'https:') throw new ShapeError(path, 'an https link');
  return text as SignInLink;
};

/** What a login last said; left out by a daemon that had nothing. */
const said = absentAs(nullable(string), null);
type EndOf<K> = Extract<LoginEnd, { end: K }>;
type SentEnd<K> = Variant<WireLoginEnd, 'end', K>;
const loginEnd = tagged<LoginEnd, WireLoginEnd, 'end'>(
  'end',
  {
    exited: object<EndOf<'exited'>, SentEnd<'exited'>>({
      end: tagValue('exited'),
      exit: processExit,
      said: nullable(string),
    }),
    cancelled: object<EndOf<'cancelled'>, SentEnd<'cancelled'>>({ end: tagValue('cancelled') }),
    timed_out: object<EndOf<'timed_out'>, SentEnd<'timed_out'>>({ end: tagValue('timed_out') }),
    broken: object<EndOf<'broken'>, SentEnd<'broken'>>({ end: tagValue('broken'), said }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { end: 'unrecognized', label: string(value.end, `${path}.end`) };
  },
);

type LoginOf<K> = Extract<Login, { kind: K }>;
type SentLogin<K> = Variant<WireLogin, 'kind', K>;
export const login = tagged<Login, WireLogin, 'kind'>(
  'kind',
  {
    starting: object<LoginOf<'starting'>, SentLogin<'starting'>>({
      kind: tagValue('starting'),
      said,
    }),
    awaiting_code: object<LoginOf<'awaiting_code'>, SentLogin<'awaiting_code'>>({
      kind: tagValue('awaiting_code'),
      link: signInLink,
      said,
    }),
    awaiting_approval: object<LoginOf<'awaiting_approval'>, SentLogin<'awaiting_approval'>>({
      kind: tagValue('awaiting_approval'),
      link: signInLink,
      code: absentAs(nullable(string), null),
      said,
    }),
    awaiting_key: object<LoginOf<'awaiting_key'>, SentLogin<'awaiting_key'>>({
      kind: tagValue('awaiting_key'),
      said,
    }),
    awaiting_redirect: object<LoginOf<'awaiting_redirect'>, SentLogin<'awaiting_redirect'>>({
      kind: tagValue('awaiting_redirect'),
      link: signInLink,
      said,
    }),
    finishing: object<LoginOf<'finishing'>, SentLogin<'finishing'>>({
      kind: tagValue('finishing'),
      link: absentAs(nullable(signInLink), null),
      said,
    }),
    // The end's own fields sit beside the login's `kind`.
    ended: (value, path) => ({ kind: 'ended', end: loginEnd(value, path) }),
  },
  (_value, path) => {
    throw new ShapeError(`${path}.kind`, 'a login state');
  },
);

export const startedSignIn = versioned(
  object<{ method: Stored<SignInMethod>; login: Login }, WireLoginBody>({
    method: signInMethod,
    login,
  }),
);
export const checkedSignIn = versioned(
  object<{ sign_in: SignInState }, WireSignInBody>({ sign_in: signInState }),
);
const loginOutcome = object<LoginOutcome, WireLoginOutcome>({ end: loginEnd, state: signInState });
type WaitOf<K> = Extract<Waited, { kind: K }>;
type SentWait<K> = Variant<WireWaited, 'kind', K>;
const waited = tagged<Waited, WireWaited, 'kind'>(
  'kind',
  {
    going: object<WaitOf<'going'>, SentWait<'going'>>({ kind: tagValue('going'), login }),
    done: object<WaitOf<'done'>, SentWait<'done'>>({
      kind: tagValue('done'),
      outcome: loginOutcome,
    }),
  },
  (_value, path) => {
    throw new ShapeError(`${path}.kind`, 'going or done');
  },
);
export const waitedSignIn = versioned(object<{ waited: Waited }, WireWaitedBody>({ waited }));
export const finishedSignIn = versioned(
  object<{ outcome: LoginOutcome }, WireOutcomeBody>({
    outcome: object<LoginOutcome, WireLoginOutcome>({ end: loginEnd, state: signInState }),
  }),
);

/** How the Sessions view words a client's sign-in. */
export class SignIns {
  /** The method's name, for the choice between them. */
  static method(method: Stored<SignInMethod>): string {
    switch (method) {
      case 'claude_subscription':
        return 'Claude subscription';
      case 'anthropic_console':
        return 'Anthropic Console';
      case 'chatgpt':
        return 'ChatGPT account';
      case 'anthropic_api_key':
        return 'Anthropic API key';
      case 'openai_api_key':
        return 'OpenAI API key';
      default:
        return unrecognised(method);
    }
  }

  /** The short hint a client picker shows; null when there is nothing to say. */
  static hint(state: SignInState): string | null {
    switch (state.kind) {
      case 'signed_in':
        return state.method;
      case 'signed_out':
        return 'signed out';
      case 'unknown':
        return 'sign-in unknown';
      case 'checking':
        return 'checking sign-in';
      case 'not_checked':
        return null;
      case 'unrecognized':
        return state.label;
      default:
        return assertNever(state);
    }
  }

  /**
   * The client a tool package is: a `client` package's entry in `clients`,
   * matched by the harness that needs it; null for any other package.
   */
  static clientOf<T extends { harness: string }>(
    pkg: { kind: string | null; needed_by: readonly string[] },
    clients: readonly T[],
  ): T | null {
    if (pkg.kind !== 'client') return null;
    return clients.find((client) => pkg.needed_by.includes(client.harness)) ?? null;
  }

  /** How a chip shows a sign-in: its words and its tone; null for none. */
  static chip(state: SignInState): { text: string; tone: 'success' | 'warning' | '' } | null {
    switch (state.kind) {
      case 'signed_in':
        return { text: state.method, tone: 'success' };
      case 'signed_out':
        return { text: 'signed out', tone: 'warning' };
      case 'unknown':
        return { text: 'sign-in unknown', tone: '' };
      case 'checking':
        return { text: 'checking sign-in', tone: '' };
      case 'not_checked':
        return null;
      case 'unrecognized':
        return { text: state.label, tone: '' };
      default:
        return assertNever(state);
    }
  }

  /** Whether the client must be signed in before a prompt can work. */
  static signedOut(state: SignInState): boolean {
    return state.kind === 'signed_out';
  }

  /** A client's words as a sentence: ended with a full stop when they are not. */
  static sentence(text: string): string {
    const trimmed = text.trim();
    return /[.!?…]$/.test(trimmed) ? trimmed : `${trimmed}.`;
  }

  /** What a login's outcome tells the operator. */
  static outcome(outcome: LoginOutcome, client: string): { ok: boolean; text: string } {
    if (outcome.state.kind === 'signed_in') {
      return { ok: true, text: `${client} is signed in (${outcome.state.method}).` };
    }
    const after =
      outcome.state.kind === 'signed_out'
        ? `${client} is still signed out.`
        : outcome.state.kind === 'unknown'
          ? `Whether ${client} is signed in is unknown: ${outcome.state.reason}`
          : `${client}'s sign-in could not be confirmed.`;
    const end = outcome.end;
    switch (end.end) {
      case 'exited':
        return { ok: false, text: end.said ? `${SignIns.sentence(end.said)} ${after}` : after };
      case 'cancelled':
        return { ok: false, text: `The sign-in was cancelled. ${after}` };
      case 'timed_out':
        return { ok: false, text: `The sign-in took too long and was stopped. ${after}` };
      case 'broken':
        return {
          ok: false,
          text: end.said
            ? `${SignIns.sentence(end.said)} ${after}`
            : `The sign-in stopped unexpectedly. ${after}`,
        };
      case 'unrecognized':
        return { ok: false, text: after };
      default:
        return assertNever(end);
    }
  }
}
