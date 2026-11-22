// The desk's one action at a time: whether one is on its way, and why the
// last one failed, said above the composer.
import { RunCommandError } from '#lib/runs.ts';

export class Acting {
  /** An action is on its way: the controls that start another wait. */
  busy = $state(false);
  /** Why the last action failed; null when it did not. */
  notice = $state<string | null>(null);

  /** Run `action`, clearing the last failure and saying this one's. */
  async run(action: () => Promise<unknown>): Promise<void> {
    this.busy = true;
    this.notice = null;
    try {
      await action();
    } catch (error) {
      this.notice = RunCommandError.say(error);
    } finally {
      this.busy = false;
    }
  }

  /** Say why something done outside `run` failed. */
  fail(error: unknown) {
    this.notice = RunCommandError.say(error);
  }
}
