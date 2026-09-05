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

export function createListCursor(options: ListCursorOptions): ListCursor {
  let index = $state(0);

  const clamp = (value: number): number => clampIndex(value, options.count());

  const reduced = (): boolean =>
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;

  function moveTo(next: number): void {
    index = clamp(next);
    const element = options.elFor(index);
    if (!element) return;
    // preventScroll, then scrollIntoView: focus() alone jumps the row to the top of the
    // viewport, which loses the rows above it that give the selection its context.
    element.focus({ preventScroll: true });
    element.scrollIntoView({ block: "nearest", behavior: reduced() ? "auto" : "smooth" });
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
