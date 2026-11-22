// Every time a page says: how long ago something was (`Elapsed.ago`), how
// long something took (`span`, `latency`, `stopwatch`), and when it was
// (`Clock`). Pure: the clock is always handed in; tested in tests/time.test.ts.

export class Elapsed {
  static readonly SECOND = 1000;
  static readonly MINUTE = 60 * Elapsed.SECOND;
  static readonly HOUR = 60 * Elapsed.MINUTE;
  static readonly DAY = 24 * Elapsed.HOUR;

  /** How long ago `at` was, short: `now`, `12s`, `4m`, `3h`, then its day
   * (`day` names one; by default its weekday within the week, else its date). */
  static ago(at: number, now: number, day: (ms: number) => string = (ms) => Clock.day(ms, now)) {
    const seconds = Math.floor(Math.max(0, now - at) / Elapsed.SECOND);
    if (seconds < 5) return 'now';
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h`;
    return day(at);
  }

  /** A length of time: `12 s`, `4 min`, `1 h 20 min`, then `2.5 days` from
   * two days on. */
  static span(ms: number): string {
    const seconds = Math.max(0, Math.round(ms / Elapsed.SECOND));
    if (seconds < 60) return `${seconds} s`;
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes} min`;
    const hours = Math.floor(minutes / 60);
    if (hours < 48) return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
    const days = ms / Elapsed.DAY;
    return `${Number.isInteger(days) ? days : days.toFixed(1)} days`;
  }

  /** A call's latency: `120 ms`, `1.25 s`; an em dash when unknown. */
  static latency(ms: number | null): string {
    if (ms === null) return '—';
    return ms < Elapsed.SECOND ? `${Math.round(ms)} ms` : `${(ms / Elapsed.SECOND).toFixed(2)} s`;
  }

  /** A step's time, as a stopwatch reads: `1.2 s`, `12 s`, `1:05`. */
  static stopwatch(ms: number): string {
    const seconds = Math.max(0, ms) / Elapsed.SECOND;
    if (seconds < 10) return `${seconds.toFixed(1)} s`;
    if (seconds < 60) return `${Math.round(seconds)} s`;
    const whole = Math.round(seconds);
    return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
  }
}

/** When something was, in the browser's own time zone and locale. */
export class Clock {
  /** Its time of day, to the second. */
  static second(ms: number): string {
    return new Date(ms).toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  }

  /** Its time of day, to the minute. */
  static minute(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  /** Its day: the weekday within the week before `now`, else its date. */
  static day(ms: number, now: number): string {
    const within = now - ms < 6 * Elapsed.DAY;
    return new Date(ms).toLocaleDateString(
      [],
      within ? { weekday: 'short' } : { month: 'short', day: 'numeric' },
    );
  }
}
