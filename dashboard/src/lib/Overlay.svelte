<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  let {
    title,
    eyebrow,
    onClose,
    busy = false,
    width = "520px",
    children,
  }: {
    title: string;
    eyebrow?: string;
    onClose: () => void;
    /** While a save is in flight, Escape and the backdrop stop closing the sheet. */
    busy?: boolean;
    /** Sheet width before the viewport caps it. A form with two columns asks for more. */
    width?: string;
    children: Snippet;
  } = $props();

  const titleId = `overlay-title-${Math.random().toString(36).slice(2, 9)}`;
  let sheet: HTMLDivElement;

  onMount(() => sheet.focus());

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !busy) {
      onClose();
      return;
    }
    if (event.key !== "Tab" || !sheet) return;

    // A modal that lets Tab walk out behind its own backdrop is modal to a mouse only.
    // The sheet itself is tabindex="-1" and focused on mount, so without this the first
    // Tab left the dialog entirely and the reader had no way back except Escape.
    const stops = [...sheet.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (element) => element.offsetParent !== null,
    );
    if (stops.length === 0) {
      event.preventDefault();
      return;
    }
    const first = stops[0];
    const last = stops[stops.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === sheet)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="overlay">
  <button class="backdrop" aria-label="Close dialog" onclick={() => !busy && onClose()}></button>
  <div
    class="sheet"
    style={`--overlay-width: ${width}`}
    bind:this={sheet}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
  >
    <div class="heading">
      <div>
        {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
        <h2 id={titleId}>{title}</h2>
      </div>
      <button class="close" aria-label="Close dialog" onclick={onClose}>×</button>
    </div>

    {@render children()}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    border: 0;
    background: rgba(0, 0, 0, 0.48);
    cursor: default;
    animation: fade-in 0.15s ease-out;
  }

  .sheet {
    position: relative;
    width: min(var(--overlay-width), 100%);
    max-height: 90vh;
    overflow-y: auto;
    padding: 24px;
    border: 1px solid var(--card-border);
    border-radius: 14px;
    background: var(--card-bg);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
  }

  .sheet:focus {
    outline: none;
  }

  .heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 18px;
  }

  /* Sentence case: the eyebrow says which section this sheet belongs to, and a
     tracked-out all-caps line above every heading is template chrome. */
  .eyebrow {
    margin: 0 0 3px;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    font-weight: 600;
  }

  h2 {
    margin: 0;
    font-size: 1.125rem;
  }

  .close {
    width: 30px;
    height: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--surface);
    color: var(--text-primary);
    font-size: 1.25rem;
    line-height: 1;
    cursor: pointer;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @media (max-width: 560px) {
    .sheet {
      padding: 20px;
    }
  }
</style>
