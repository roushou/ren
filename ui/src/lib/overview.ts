// The overview page's pure logic: the health line, what needs attention,
// and which sessions are active. No browser needed.
import type { CallView, ExposureView, FileUsage, Health, SessionView, Storage } from './api.ts';
import { bytes } from './api.ts';
import { buildLabel, callStatus, statusBadge, type Chip } from './display.ts';

/** A health chip and where to look when it is not fine. */
export interface HealthItem extends Chip {
  href?: string;
}

const live = (file: FileUsage) =>
  Math.max(0, file.page_count - file.freelist_count) * file.page_size;

/**
 * One line that says whether everything is fine: the daemon is reachable,
 * recording works, calls are admitted, and storage has room. Each item is
 * green when fine and says what is wrong when not.
 */
export function healthLine(
  health: Health | null,
  storage: Storage | null,
  error: string | null,
  exposures: ExposureView[] | null = null,
): HealthItem[] {
  if (!health)
    return [
      {
        label: "Can't reach TGV",
        tone: 'error',
        title: error ?? 'The daemon did not answer.',
      },
    ];
  const items: HealthItem[] = [
    { label: 'Daemon reachable', tone: 'success', title: buildLabel(health.build) },
  ];
  const recorder = health.recorder;
  const lost =
    recorder.omissions.unrecorded_calls +
    recorder.omissions.queue_bytes_bytes +
    recorder.omissions.queue_slots_bytes +
    recorder.omissions.storage_failure_bytes;
  items.push(
    health.status !== 'ok' || recorder.storage_errors || recorder.retention_errors
      ? {
          label: 'Recording failing',
          tone: 'error',
          title: 'Storage or retention errors since the daemon started',
          href: '/recorder',
        }
      : lost
        ? {
            label: 'Capture loss',
            tone: 'warning',
            title: 'Some calls or body bytes were not recorded since the daemon started',
            href: '/recorder',
          }
        : {
            label: 'Recording',
            tone: 'success',
            title: 'No capture loss since the daemon started',
          },
  );
  const operator = health.operator;
  if (operator) {
    const pause = operator.pause;
    switch (pause.state) {
      case 'running':
        items.push({ label: 'Admitting calls', tone: 'success', title: 'Not paused' });
        break;
      case 'paused':
        items.push({
          label: `Paused · retry in ${pause.retry_after} s`,
          tone: 'warning',
          title: 'Every new call gets 503; `tgv control resume` admits them again',
        });
        break;
      case 'unknown':
        items.push({
          label: pause.text,
          tone: 'neutral',
          title: 'An admission state this UI does not know',
        });
        break;
    }
    if (operator.blocked_sessions)
      items.push({
        label: `${operator.blocked_sessions} session${operator.blocked_sessions === 1 ? '' : 's'} blocked`,
        tone: 'warning',
        title: 'Calls from blocked sessions get 403; `tgv control unblock` lifts a block',
      });
  }
  if (storage) {
    const used = live(storage.usage.disk.metadata) + live(storage.usage.disk.blobs);
    const budget = storage.policy.max_storage_bytes;
    const share = storage.policy.enabled && budget ? used / budget : null;
    items.push(
      share === null
        ? {
            label: `${bytes(used)} stored`,
            tone: 'neutral',
            title: 'Retention is off: no storage limit',
            href: '/recorder',
          }
        : {
            label: `${bytes(used)} of ${bytes(budget)}`,
            tone: share >= 0.9 ? 'error' : share >= 0.7 ? 'warning' : 'success',
            title:
              'Live storage against the retention budget; the oldest calls are removed past it',
            href: '/recorder',
          },
    );
  }
  if (exposures) {
    const newest = exposures[0];
    items.push(
      newest
        ? {
            label: `${exposures.length}${exposures.length === 100 ? '+' : ''} secret${exposures.length === 1 ? '' : 's'} exposed · 24 h`,
            tone: 'error',
            title: 'Credentials in well-known formats found in calls; rotate them',
            href: newest.session
              ? `/sessions/${encodeURIComponent(newest.session)}/report`
              : `/traffic?call=${encodeURIComponent(newest.call)}`,
          }
        : {
            label: 'No secrets seen · 24 h',
            tone: 'success',
            title: 'No credentials in well-known formats found in the last day',
          },
    );
  }
  return items;
}

/** A call worth a look, and why. */
export interface Problem {
  call: CallView;
  reason: Chip;
}

/**
 * Errors and stopped calls, newest first, each once, at most `limit`. A
 * call can appear in both listings (an error status on a stream that was
 * also cut off); it is shown once, with its row badge as the reason.
 */
export function problems(errors: CallView[], stopped: CallView[], limit = 10): Problem[] {
  const seen = new Set<string>();
  return [...errors, ...stopped]
    .filter((call) => (seen.has(call.id) ? false : (seen.add(call.id), true)))
    .sort((a, b) => b.started_at - a.started_at)
    .slice(0, limit)
    .map((call) => {
      const badge = statusBadge(callStatus(call));
      return {
        call,
        reason: {
          label:
            call.status !== null && call.status >= 400
              ? `${badge.label} ${call.status}`
              : badge.label,
          tone: badge.tone,
          title: 'How the call ended',
        },
      };
    });
}

/** Sessions with a call in the last `window` milliseconds, most recent first. */
export function activeSessions(sessions: SessionView[], now: number, window = 15 * 60_000) {
  return sessions
    .filter((session) => now - session.last_seen <= window)
    .sort((a, b) => b.last_seen - a.last_seen);
}

/** `just now`, `45 s ago`, `12 min ago`, `3 h ago`. */
export function ago(ms: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds} s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  return `${Math.round(minutes / 60)} h ago`;
}
