// Answering an approval from outside its session (Pending, the desk): which
// one is under review, whether an answer is on its way, and what went
// wrong with the last. The answer goes to the session's run, and comes back
// as a toast that leads to the session. `ApprovalModal.svelte` draws the
// review.

import { invalidateAll } from '$app/navigation';
import { toasts } from './layer/toasts.svelte.ts';
import type { Approval } from './projects.ts';
import { RunCommandError, Runs, RunsClient, Sessions, type Decision } from './runs.ts';

export class ApprovalAnswers {
  /** The approval under review; null when none is. */
  reviewing = $state<Approval | null>(null);
  answering = $state(false);
  failed = $state<string | null>(null);
  readonly #runs = new RunsClient();

  review(waiting: Approval): void {
    this.failed = null;
    this.reviewing = waiting;
  }

  close(): void {
    this.reviewing = null;
  }

  async answer(waiting: Approval, decision: Decision): Promise<void> {
    this.answering = true;
    this.failed = null;
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
      this.reviewing = null;
      await invalidateAll();
    } catch (failure) {
      this.failed = failure instanceof RunCommandError ? failure.message : 'The answer did not go.';
    } finally {
      this.answering = false;
    }
  }

  /** Where an approval comes from: its project, and its session's prompt. */
  static origin(waiting: Approval): string {
    return [waiting.project ?? 'No project', waiting.prompt]
      .filter((part) => part !== null)
      .join(' · ');
  }
}
