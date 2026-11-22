// The bottom edge of every page: silent while everything is well, one
// strip when something is not. Each strip says the problem in words with
// the one thing to do; the worst shows, the rest wait behind "+N". Pure:
// the layout passes in what it read.

/** What one strip says. */
export type Strip =
  /** Ren does not answer: the page shows what was read last, if anything. */
  | { kind: 'unreachable'; readAt: number | null }
  /** Ren answers in an API this UI does not speak. */
  | { kind: 'incompatible' }
  /** The recorder lost calls or bytes, or recorded errors: until dismissed. */
  | { kind: 'recorder'; said: string }
  /** This UI holds no control token: a setup, always said. */
  | { kind: 'readOnly' };

/** How bad a strip is, which orders them and colours them. */
export type StripTone = 'bad' | 'warning' | 'plain';

/** What the layout read, as the edge needs it. */
export interface EdgeReading {
  /** Why Ren's reads fail, or null when they do not. */
  failure: 'unreachable' | 'incompatible' | null;
  /** When Ren was last read whole. */
  readAt: number | null;
  /** What the recorder lost or got wrong, in words; null when nothing. */
  recorder: string | null;
  /** Recorder notices the operator dismissed, by their words. */
  dismissed: ReadonlySet<string>;
  readOnly: boolean;
}

export class BottomEdge {
  /** The strips to say, worst first; none while everything is well. */
  static strips(reading: EdgeReading): Strip[] {
    const strips: Strip[] = [];
    if (reading.failure === 'incompatible') strips.push({ kind: 'incompatible' });
    if (reading.failure === 'unreachable')
      strips.push({ kind: 'unreachable', readAt: reading.readAt });
    if (reading.recorder !== null && !reading.dismissed.has(reading.recorder))
      strips.push({ kind: 'recorder', said: reading.recorder });
    if (reading.readOnly) strips.push({ kind: 'readOnly' });
    return strips;
  }

  static tone(strip: Strip): StripTone {
    switch (strip.kind) {
      case 'unreachable':
      case 'incompatible':
        return 'bad';
      case 'recorder':
        return 'warning';
      case 'readOnly':
        return 'plain';
      default: {
        const never: never = strip;
        return never;
      }
    }
  }
}
