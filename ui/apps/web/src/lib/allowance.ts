// How much of a provider's allowance is left, read from the limit headers on
// its newest response: a subscription's windows (Anthropic's unified
// utilization, Codex's used percent) or an API key's remaining requests and
// tokens. The tightest one says it. Pure; tested in tests/allowance.test.ts.
import type { LimitHeader } from './api.ts';
import { compact } from './display.ts';

/** What is left of one limit. */
export interface Allowance {
  /** What it limits: `5 h window`, `requests`, `input tokens`. */
  name: string;
  /** Left, 0–1. */
  left: number;
  text: string;
}

export class Allowances {
  /** Less than this share of a limit left needs a look. */
  static readonly LOW = 0.2;

  /** The tightest limit the headers state; null when they state none. */
  static read(headers: readonly LimitHeader[]): Allowance | null {
    return Allowances.all(headers).reduce<Allowance | null>(
      (tightest, each) => (!tightest || each.left < tightest.left ? each : tightest),
      null,
    );
  }

  /** Every limit the headers state, in the order they came. */
  static all(headers: readonly LimitHeader[]): Allowance[] {
    const values = new Map<string, string>();
    for (const header of headers)
      if (header.value !== null) values.set(header.name.toLowerCase(), header.value.trim());
    const found: Allowance[] = [];
    const counts = new Map<string, { remaining?: number; limit?: number }>();
    const count = (unit: string, part: 'remaining' | 'limit', value: string) => {
      const parsed = Number(value);
      if (!Number.isFinite(parsed)) return;
      counts.set(unit, { ...counts.get(unit), [part]: parsed });
    };
    for (const [name, value] of values) {
      const unified = /^anthropic-ratelimit-unified-([0-9]+[hd])-utilization$/.exec(name);
      if (unified) {
        const used = Number(value);
        if (Number.isFinite(used)) found.push(Allowances.window(used, Allowances.span(unified[1])));
        continue;
      }
      const codex = /^x-codex-(primary|secondary)-used-percent$/.exec(name);
      if (codex) {
        const used = Number(value) / 100;
        const minutes = Number(values.get(`x-codex-${codex[1]}-window-minutes`));
        if (Number.isFinite(used))
          found.push(
            Allowances.window(used, Number.isFinite(minutes) ? Allowances.minutes(minutes) : null),
          );
        continue;
      }
      const anthropic =
        /^anthropic-ratelimit-(requests|tokens|input-tokens|output-tokens)-(remaining|limit)$/.exec(
          name,
        );
      if (anthropic) {
        count(anthropic[1], anthropic[2] as 'remaining' | 'limit', value);
        continue;
      }
      const openai = /^x-ratelimit-(remaining|limit)-(requests|tokens)$/.exec(name);
      if (openai) count(openai[2], openai[1] as 'remaining' | 'limit', value);
    }
    for (const [unit, { remaining, limit }] of counts)
      if (remaining !== undefined && limit)
        found.push({
          name: unit.replace('-', ' '),
          left: Math.min(1, Math.max(0, remaining / limit)),
          text: `${compact(remaining)} / ${compact(limit)} ${unit.replace('-', ' ')}`,
        });
    return found;
  }

  static window(used: number, span: string | null): Allowance {
    const share = Math.min(1, Math.max(0, used));
    return {
      name: span ? `${span} window` : 'window',
      left: 1 - share,
      text: `${Math.round(share * 100)}% used${span ? ` · ${span}` : ''}`,
    };
  }

  /** `5h` → `5 h`, `7d` → `7 d`. */
  static span(window: string): string {
    return `${window.slice(0, -1)} ${window.slice(-1)}`;
  }

  /** A window's length in minutes, as a span: `5 h`, `7 d`, `30 min`. */
  static minutes(minutes: number): string {
    if (minutes % 1440 === 0) return `${minutes / 1440} d`;
    if (minutes % 60 === 0) return `${minutes / 60} h`;
    return `${minutes} min`;
  }
}
