/**
 * What makes a `role="dialog"` modal to a keyboard, in one place.
 *
 * The three behaviours below are not decoration and not independent — a dialog with any
 * one of them missing is a dialog a reader can get lost in:
 *
 *   focus in     the sheet takes focus on mount, so the next keystroke belongs to it and
 *                a screen reader reads the dialog's label rather than the page behind it;
 *   trap         Tab and Shift+Tab wrap inside the sheet. Without this the first Tab
 *                walks out behind the backdrop into a page the reader cannot see, and
 *                Escape is the only way back — modal to a mouse, not to a keyboard;
 *   focus out    on close, focus returns to whatever opened the dialog. Without this it
 *                falls to `<body>`, and the reader restarts at the top of the document —
 *                every tab stop between the page header and the control they pressed has
 *                to be walked again, on a page they have already read.
 *
 * Measured on 2026-09-08, before this file existed: four `role="dialog" aria-modal="true"`
 * sheets in `dashboard/src/`; one (`$lib/Overlay.svelte`) trapped Tab, three did not, and
 * none of the four restored focus. `$lib/Overlay.svelte` is where the trap below comes
 * from — it is moved, not invented.
 *
 * Escape is handled here too, and takes a `canClose` predicate rather than a boolean:
 * every one of these sheets can be mid-save, and a dialog that vanishes under an in-flight
 * request loses the operator's answer with no record that it was ever given.
 */

import type { Action } from "svelte/action";

/**
 * Everything a Tab press can reach.
 *
 * `[tabindex]:not([tabindex="-1"])` and not a bare `[tabindex]`: the sheet itself carries
 * `tabindex="-1"` so it can be focused programmatically, and including it would make the
 * container one of its own tab stops.
 */
export const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]),' +
  ' textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Where Tab should send focus, or `null` when the browser's own answer is already right.
 *
 * Pure and generic over the element type so `tools/dashboard-modal.test.ts` can drive it
 * without a DOM — `bun test` has no document, and the arithmetic is the half that is easy
 * to get subtly wrong. Returning `null` rather than the next stop matters: re-implementing
 * the browser's ordering inside the sheet is how a trap starts skipping controls.
 */
export function nextTrapStop<T>(
  stops: readonly T[],
  root: T,
  active: T | null,
  shiftKey: boolean,
): T | null {
  // Nothing to tab to. Focus stays on the sheet rather than leaving it.
  if (stops.length === 0) return root;

  const first = stops[0];
  const last = stops[stops.length - 1];

  // Focus is somewhere else entirely — the backdrop button, `<body>` after a control was
  // removed, or the page behind. Pull it back to the near end.
  if (active !== root && !stops.includes(active as T)) return shiftKey ? last : first;

  if (shiftKey) return active === first || active === root ? last : null;
  return active === last ? first : null;
}

/** The reachable tab stops inside `root`, in document order. */
export function tabStops(root: HTMLElement): HTMLElement[] {
  // `offsetParent === null` is the display:none test that survives every browser this
  // surface is read in. A control inside a collapsed branch is painted nowhere and must
  // not be a stop.
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (element) => element.offsetParent !== null,
  );
}

export interface ModalOptions {
  /** Escape was pressed. The dialog decides what closing means. */
  onClose?: () => void;
  /** False while a save is in flight. Defaults to true. */
  canClose?: () => boolean;
}

/**
 * `use:modal` on the element that carries `role="dialog"`.
 *
 * The element must also carry `tabindex="-1"`, or the mount focus has nowhere to land and
 * the reader starts the dialog on the page behind it.
 */
export const modal: Action<HTMLElement, ModalOptions | undefined> = (node, options) => {
  let current: ModalOptions = options ?? {};

  // Read before anything is focused. `document.activeElement` is `<body>` once the sheet
  // has taken focus, so recording it later records nothing.
  const opener = document.activeElement as HTMLElement | null;

  node.focus();

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      if (current.canClose?.() === false) return;
      current.onClose?.();
      return;
    }
    if (event.key !== "Tab") return;

    const target = nextTrapStop(
      tabStops(node),
      node,
      document.activeElement as HTMLElement | null,
      event.shiftKey,
    );
    if (!target) return;
    event.preventDefault();
    target.focus();
  }

  window.addEventListener("keydown", onKeydown);

  return {
    update(next) {
      current = next ?? {};
    },
    destroy() {
      window.removeEventListener("keydown", onKeydown);
      // `isConnected`, because the opener is often a row that the very action taken in
      // this dialog has just removed from the list. Focusing a detached node silently
      // sends focus to `<body>`, which is the state this exists to prevent.
      if (opener?.isConnected) opener.focus();
    },
  };
};
