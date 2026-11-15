/**
 * Where Redline's briefs can go in Ren, read from the daemon once per
 * opening of the pane: the clients ready here, and the sessions running in
 * Ren's source checkout. The layout owns it; Ren's pane reads it.
 */
import { Briefing, type Reading } from './briefing.ts';
import { DRIVABLE, RunCommandError, RunsClient } from './runs.ts';

export class Recipients {
  /** Where a brief can go, read once per opening. */
  destinations = $state.raw<Reading>({ state: 'idle' });
  /** Ren's source checkout, where briefs go; null when the UI is not told. */
  readonly #checkout: () => string | null;
  /** Whether what was read is from an earlier opening, to be read again. */
  #stale = false;

  constructor(checkout: () => string | null) {
    this.#checkout = checkout;
  }

  /** Read where a brief can go: the clients, and the sessions open in the
   * checkout. Started as the pointer nears the toggle, so the pane usually
   * opens with them; read again for each opening, keeping the last reading
   * shown meanwhile, since sessions come and go. */
  prepare() {
    const { state } = this.destinations;
    if (state === 'reading' || (state === 'read' && !this.#stale)) return;
    if (state !== 'read') this.destinations = { state: 'reading' };
    this.#stale = false;
    const checkout = this.#checkout();
    const client = new RunsClient();
    Promise.all([
      client.clients(),
      checkout ? client.sessions() : [],
      checkout ? client.live() : [],
    ]).then(
      ([clients, bodies, live]) => {
        this.destinations = {
          state: 'read',
          value: {
            ready: DRIVABLE.filter((drivable) =>
              clients.some((entry) => entry.harness === drivable && entry.status === 'ready'),
            ),
            clients,
            sessions: checkout ? Briefing.offered(bodies, live, checkout) : [],
          },
        };
      },
      (failure: unknown) => {
        this.destinations = {
          state: 'failed',
          reason:
            failure instanceof RunCommandError
              ? failure.message
              : 'Cannot read the clients and sessions.',
        };
      },
    );
  }

  /** Read the clients and sessions again now, keeping what is shown
   * meanwhile: after a client's sign-in changed. */
  reread() {
    this.#stale = true;
    this.prepare();
  }

  /** The pane closed: what was read is read again at its next opening. */
  close() {
    this.#stale = true;
  }
}
