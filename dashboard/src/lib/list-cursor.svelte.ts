/**
 * J / K / Enter over a list, as one module rather than one copy per page.
 *
 * The change from what Home did: moving the cursor moves REAL DOM focus. Home's handler
 * sits on `<svelte:window>` over a queue that is never focused, so a screen reader was
 * never told the selection had changed — a `.selected` class is a paint, not an
 * announcement. `elFor(index)?.focus()` makes it real, and it is why a row is a `listitem`
 * with `tabindex="-1"` rather than an `option`: an option must not contain focusable
 * descendants, and every row here holds a title link and one or two buttons.
 */

export interface ListCursor {
  readonly index: number;
  /** Wire to `<svelte:window onkeydown={…}>`. */
  handleKeydown(event: KeyboardEvent): void;
  moveTo(index: number): void;
}

export interface ListCursorOptions {
  /** How many rows exist right now. Read on every move, so a shrinking list clamps. */
  count(): number;
  /** The element to focus for a row, or undefined if it is not on screen yet. */
  elFor(index: number): HTMLElement | null | undefined;
  onOpen(index: number): void;
  /** Extra single-key bindings, e.g. `{ x: (i) => dismiss(i) }`. Lower-cased keys. */
  bindings?: Record<string, (index: number) => void>;
}

/** Where a keystroke belongs to the control the reader is already in, not to the list. */
export const INTERACTIVE =
  "a, button, input, select, textarea, [contenteditable], [role='dialog']";

/**
 * True when this keystroke is not the list's to handle.
 *
 * Exported and pure so `tools/dashboard-list-cursor.test.ts` can assert the guard without
 * a Svelte compiler: the runes below only exist once `createListCursor` is called.
 */
export function shouldIgnoreKey(event: KeyboardEvent): boolean {
  // Duck-typed on `closest` rather than `instanceof Element`: `window` is the target when
  // nothing has focus, which is the ordinary case, and `Element` is not a global outside a
  // browser at all — an `instanceof` against it throws in a plain test runner.
  const target = event.target as { closest?: (selector: string) => unknown } | null;
  return Boolean(
    event.metaKey || event.ctrlKey || event.altKey || target?.closest?.(INTERACTIVE),
  );
}

/** Clamped into `[0, count)`, or 0 for an empty list. */
export function clampIndex(value: number, count: number): number {
  if (count <= 0) return 0;
  return Math.min(Math.max(0, value), count - 1);
}

/**
 * One rendered line in a list that is not uniformly selectable.
 *
 * The Feed interleaves collector-run headers with items; only an item takes the cursor.
 * `clampIndex` above is the uniform case and cannot express this, so the two lists kept
 * two cursors until 2026-09-07. The arithmetic lives here, beside the guard both already
 * shared, because a page that disagrees with another page about what `j` does is the
 * defect this module exists to prevent.
 */
export interface CursorRow {
  kind: "header" | "item";
  id: string;
}

/** The first selectable row at or after `from`, walking by `step`, or -1 when there is none. */
function seek(rows: readonly CursorRow[], from: number, step: number): number {
  for (let index = from; index >= 0 && index < rows.length; index += step) {
    if (rows[index].kind === "item") return index;
  }
  return -1;
}

/**
 * Move down over selectable rows.
 *
 * Stops on the last item rather than wrapping: a wrap on a long list moves the reader
 * somewhere they did not ask to go.
 */
export function nextSelectable(rows: readonly CursorRow[], index: number): number {
  const found = seek(rows, index + 1, 1);
  return found === -1 ? (rows[index]?.kind === "item" ? index : seek(rows, 0, 1)) : found;
}

/** Move up, stopping on the first item. */
export function prevSelectable(rows: readonly CursorRow[], index: number): number {
  const found = seek(rows, index - 1, -1);
  return found === -1 ? (rows[index]?.kind === "item" ? index : seek(rows, 0, 1)) : found;
}

/**
 * Where the cursor sits after the list changed under it.
 *
 * A decided row is greyed in place rather than removed, so most of the time this returns
 * the same index. It earns its place on the reload that does remove the row: without it
 * the cursor lands on a header, or past the end, and the next keystroke does nothing.
 */
export function clampToSelectable(rows: readonly CursorRow[], index: number): number {
  if (rows.length === 0) return -1;
  const bounded = Math.min(Math.max(index, 0), rows.length - 1);
  if (rows[bounded].kind === "item") return bounded;
  const forward = seek(rows, bounded, 1);
  return forward === -1 ? seek(rows, bounded, -1) : forward;
}

/** The reader asked the operating system for less motion. */
function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/**
 * Put the cursor on a row, for real.
 *
 * Exported rather than kept inside `createListCursor` because a page whose rows are not
 * uniformly selectable cannot use the factory — /feed interleaves collector-run headers
 * with items and drives `nextSelectable`/`prevSelectable` itself — and it spent three
 * months scrolling a row into view without focusing it. That is a paint: `aria-current`
 * moved and no assistive technology was told, so the whole j/k lane was silent to a
 * screen reader and left a keyboard reader's Tab position wherever it had been.
 *
 * The order is load-bearing. `focus()` on its own scrolls the row to the top of the
 * viewport, which throws away the rows above it that give a selection its context, so the
 * scroll is suppressed and then asked for again as `block: "nearest"`.
 */
export function focusRow(element: HTMLElement | null | undefined): void {
  if (!element) return;
  element.focus({ preventScroll: true });
  element.scrollIntoView({ block: "nearest", behavior: reducedMotion() ? "auto" : "smooth" });
}

export function createListCursor(options: ListCursorOptions): ListCursor {
  let index = $state(0);

  const clamp = (value: number): number => clampIndex(value, options.count());

  function moveTo(next: number): void {
    index = clamp(next);
    focusRow(options.elFor(index));
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (shouldIgnoreKey(event)) return;

    const count = options.count();
    const key = event.key.toLowerCase();

    if (key === "j" && count > 0) {
      event.preventDefault();
      moveTo(index + 1);
    } else if (key === "k" && count > 0) {
      event.preventDefault();
      moveTo(index - 1);
    } else if (event.key === "Enter" && count > 0) {
      event.preventDefault();
      options.onOpen(clamp(index));
    } else if (options.bindings?.[key] && count > 0) {
      event.preventDefault();
      options.bindings[key](clamp(index));
    }
  }

  return {
    get index() {
      // Clamped on read as well as on move: the list can shrink under the cursor when a
      // dismissal resolves, and nothing dispatches an event when it does.
      return clamp(index);
    },
    handleKeydown,
    moveTo,
  };
}
