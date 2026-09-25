/**
 * The words and the diff behind the app's sync status line and conflicts view
 * (`./SyncStatus.svelte`). Pure functions, so `vite/sync-status.test.ts` runs them without Tauri.
 *
 * The state they read comes from the device's local store (`src-tauri/src/sync.rs`). The canonical
 * node stays the authority for this phase: a conflict shows both values and a person picks one.
 */
import type { OutboxEntry, SyncStatus } from './mac-bridge';

/** `HH:MM` in the device's time zone. */
export function clockTime(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/**
 * The parts of the status line, most urgent first. Empty when there is nothing to say: the
 * canonical node answers and nothing waits.
 */
export function statusParts(status: SyncStatus, time: (ms: number) => string = clockTime): string[] {
  const parts: string[] = [];
  if (status.offline) {
    parts.push(
      status.showing_from !== null
        ? `Offline — showing data from ${time(status.showing_from)}`
        : 'Offline — the canonical node does not answer',
    );
  }
  if (status.conflicts > 0) parts.push(plural(status.conflicts, 'conflict', 'conflicts'));
  if (status.failed > 0) parts.push(`${plural(status.failed, 'change', 'changes')} refused by the canonical node`);
  if (status.pending > 0) parts.push(`${plural(status.pending, 'change', 'changes')} waiting`);
  if (status.store_error) parts.push(`Offline copy unavailable: ${status.store_error}`);
  return parts;
}

export function statusLine(status: SyncStatus, time?: (ms: number) => string): string | null {
  const parts = statusParts(status, time);
  return parts.length > 0 ? parts.join(' · ') : null;
}

/** True when something needs a person: a conflict or a refused change. */
export function needsReview(status: SyncStatus): boolean {
  return status.conflicts > 0 || status.failed > 0;
}

/** One field of a queued edit next to the Mac's value. */
export interface FieldDiff {
  field: string;
  mine: unknown;
  theirs: unknown;
}

/** Fields the server owns or reads as a condition, never as an edit. */
const NOT_A_FIELD = new Set(['id', 'revision', 'expected_revision', 'pending', 'conflict']);

/** `undefined` and `null` both mean "no value"; the rest compares as JSON. */
function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/**
 * The fields where the operator's queued edit and the canonical node's current item differ. A field the
 * edit does not name is not shown: the edit leaves it alone (PATCH), or the canonical node's value stands
 * anyway once the operator picks. Without the Mac's item every named field is shown with
 * `theirs: null`.
 */
export function fieldDiff(entry: Pick<OutboxEntry, 'body' | 'current'>): FieldDiff[] {
  const theirs = entry.current?.item ?? {};
  return Object.entries(entry.body)
    .filter(([field]) => !NOT_A_FIELD.has(field))
    .filter(([field, mine]) => !same(mine, theirs[field]))
    .map(([field, mine]) => ({ field, mine: mine ?? null, theirs: theirs[field] ?? null }));
}

/** A value as the conflicts view prints it. */
export function showValue(value: unknown): string {
  if (value === null || value === undefined || value === '') return '—';
  if (typeof value === 'string') return value;
  return JSON.stringify(value);
}

/** Fired on `window` after the outbox changed, so a page can reload what it shows. */
export const SYNC_CHANGED_EVENT = 'axon-sync-changed';
