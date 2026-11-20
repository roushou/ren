// The UI's marks: one glyph for each state, outcome, and severity a view
// shows, so a fixed word reads at a glance and the word itself moves into
// the glyph's tooltip. `Glyph.svelte` draws them; each has its own shape, so
// colour is never the only cue. Pure: no DOM.

import { assertNever } from './decode.ts';

/** Every mark a view can draw. */
export const marks = [
  /** Whose move it is. */
  'responding',
  'waiting',
  'slow',
  'client',
  'yours',
  'idle',
  'failed',
  'warning',
  'unknown',
  /** How something ended, or that it has not yet. */
  'passed',
  'running',
  'queued',
  'draft',
  /** A finding that asks for nothing. */
  'info',
] as const;
export type Mark = (typeof marks)[number];

export class Marks {
  /** A mark's word, for its tooltip and screen readers, when the view has
   * none more specific to say. */
  static word(mark: Mark): string {
    switch (mark) {
      case 'responding':
        return 'Responding';
      case 'waiting':
        return 'Waiting for a response';
      case 'slow':
        return 'Slow to respond';
      case 'client':
        return 'Running a tool';
      case 'yours':
        return 'Waits for you';
      case 'idle':
        return 'Idle';
      case 'failed':
        return 'Failed';
      case 'warning':
        return 'Needs a look';
      case 'unknown':
        return 'Unknown';
      case 'passed':
        return 'Passed';
      case 'running':
        return 'Running';
      case 'queued':
        return 'Queued';
      case 'draft':
        return 'Draft';
      case 'info':
        return 'Information';
      default:
        return assertNever(mark);
    }
  }
}
