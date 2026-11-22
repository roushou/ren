// Answering an approval from outside its session (Pending, the desk): which
// one is under review, whether an answer is on its way, and what went
// wrong with the last. The answer goes to the session's run, and comes back
// as a toast that leads to the session. `ApprovalModal.svelte` draws the
// review.

import { invalidateAll } from '$app/navigation';
import { toasts } from './layer/toasts.svelte.ts';
import type { Approval } from './projects.ts';
import { RunCommandError, Runs, RunsClient, Sessions, type Decision } from './runs.ts';

/** Where answering stands: nothing under review, one under review, its
 * answer on its way, or why the last one did not go. */
export type Answering =
  | { kind: 'idle' }
  | { kind: 'reviewing'; approval: Approval }
  | { kind: 'answering'; approval: Approval }
  | { kind: 'failed'; approval: Approval; why: string };

export class ApprovalAnswers {
  state = $state.raw<Answering>({ kind: 'idle' });
  readonly #runs = new RunsClient();

  /** The approval under review, answering or failed; null when none is. */
  get reviewing(): Approval | null {
    return this.state.kind === 'idle' ? null : this.state.approval;
  }

  get answering(): boolean {
    return this.state.kind === 'answering';
  }

  /** Why the last answer did not go. */
  get failed(): string | null {
    return this.state.kind === 'failed' ? this.state.why : null;
  }

  review(waiting: Approval): void {
    this.state = { kind: 'reviewing', approval: waiting };
  }

  close(): void {
    this.state = { kind: 'idle' };
  }

  async answer(waiting: Approval, decision: Decision): Promise<void> {
    this.state = { kind: 'answering', approval: waiting };
    try {
      await this.#runs.toSession(waiting.session, {
        kind: 'answer',
        approval: waiting.approval,
        decision,
      });
      const said = Runs.decision(decision);
      const asked = Runs.ask(waiting.ask);
      toasts.show({
        mark: said.mark,
        text: asked.detail ?? asked.action,
        form: asked.detail ? 'code' : 'words',
        detail: [said.done, waiting.project].filter((part) => part !== null).join(' · '),
        link: {
          label: 'Open session',
          href: Sessions.deskHref(waiting.project, waiting.session),
        },
      });
      this.state = { kind: 'idle' };
      await invalidateAll();
    } catch (failure) {
      this.state = {
        kind: 'failed',
        approval: waiting,
        why: failure instanceof RunCommandError ? failure.message : 'The answer did not go.',
      };
    }
  }

  /** Where an approval comes from: its project, and its session's prompt. */
  static origin(waiting: Approval): string {
    return [waiting.project ?? 'No project', waiting.prompt]
      .filter((part) => part !== null)
      .join(' · ');
  }
}
