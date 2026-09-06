// Keyboard cursor arithmetic for the Feed inbox.
//
// A plain module, no runes. `bunfig.toml` declares no Svelte plugin for its test
// runner, so a `.svelte.ts` importing `$state` fails at module evaluation inside a
// bare `bun test`. It is also the honest shape: the cursor is arithmetic over a
// list. The `$state` index, `scrollIntoView` and the DOM ids stay in the page.

/** One rendered line. A header is a collector-run label; only items are selectable. */
export interface CursorRow {
  kind: 'header' | 'item';
  id: string;
}

/** The first selectable row at or after `from`, or -1 when there is none. */
function seek(rows: readonly CursorRow[], from: number, step: number): number {
  for (let index = from; index >= 0 && index < rows.length; index += step) {
    if (rows[index].kind === 'item') return index;
  }
  return -1;
}

/** Move down. Stops on the last item rather than wrapping: a wrap on a long list
 *  moves the reader somewhere they did not ask to go. */
export function next(rows: readonly CursorRow[], index: number): number {
  const found = seek(rows, index + 1, 1);
  return found === -1 ? (rows[index]?.kind === 'item' ? index : seek(rows, 0, 1)) : found;
}

/** Move up, stopping on the first item. */
export function prev(rows: readonly CursorRow[], index: number): number {
  const found = seek(rows, index - 1, -1);
  return found === -1 ? (rows[index]?.kind === 'item' ? index : seek(rows, 0, 1)) : found;
}

/**
 * Where the cursor sits after the list changed under it.
 *
 * A decided row is greyed in place rather than removed, so most of the time this
 * returns the same index. It earns its place on the reload that does remove the
 * row: without it the cursor lands on a header, or past the end, and the next
 * keystroke does nothing.
 */
export function clampAfterDecision(rows: readonly CursorRow[], index: number): number {
  if (rows.length === 0) return -1;
  const bounded = Math.min(Math.max(index, 0), rows.length - 1);
  if (rows[bounded].kind === 'item') return bounded;
  const forward = seek(rows, bounded, 1);
  return forward === -1 ? seek(rows, bounded, -1) : forward;
}

/**
 * Whether a keystroke belongs to the thing the reader is typing in.
 *
 * Lifted verbatim from Home's own guard, because two pages disagreeing about
 * which keys are shortcuts is worse than either rule: bail on a modifier, and on
 * any target inside a link, a button or a form control.
 */
export function isTypingTarget(event: KeyboardEvent): boolean {
  if (event.metaKey || event.ctrlKey || event.altKey) return true;
  const target = event.target as HTMLElement | null;
  return Boolean(target?.closest?.('a, button, input, select, textarea'));
}
