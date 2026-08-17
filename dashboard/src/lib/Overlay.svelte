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

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !busy) onClose();
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

  .eyebrow {
    margin: 0 0 3px;
    color: var(--text-secondary);
    font-size: 0.6875rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
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
