<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { base } from '$app/paths';
  import { page } from '$app/state';
  import Icon from '$lib/Icon.svelte';
  import { assistantStore } from './assistant.svelte';
  import { extractRouteContext } from './context';
  import JourneyActionCard from './cards/JourneyActionCard.svelte';
  import CalendarActionCard from './cards/CalendarActionCard.svelte';

  let inputPrompt = $state('');
  let chatBodyEl: HTMLElement | undefined = $state();
  let inputEl: HTMLTextAreaElement | undefined = $state();
  let drawerEl: HTMLElement | undefined = $state();

  let touchStartY = 0;
  let touchDeltaY = $state(0);

  // The route without the demo base (AXON_DEMO_BASE), so /travel is /travel on Pages too.
  const routePath = $derived(
    base && page.url.pathname.startsWith(base) ? page.url.pathname.slice(base.length) || '/' : page.url.pathname,
  );
  const currentContext = $derived(extractRouteContext(routePath));

  function handleTouchStart(e: TouchEvent) {
    if (e.touches.length === 1) {
      touchStartY = e.touches[0].clientY;
    }
  }

  function handleTouchMove(e: TouchEvent) {
    if (e.touches.length === 1) {
      const delta = e.touches[0].clientY - touchStartY;
      if (delta > 0) {
        touchDeltaY = delta;
      }
    }
  }

  function handleTouchEnd() {
    if (touchDeltaY > 110) {
      assistantStore.closeDrawer();
    }
    touchDeltaY = 0;
  }

  async function scrollToBottom() {
    await tick();
    if (chatBodyEl) {
      chatBodyEl.scrollTop = chatBodyEl.scrollHeight;
    }
  }

  async function handleSend() {
    const text = inputPrompt.trim();
    if (!text || assistantStore.loading) return;

    inputPrompt = '';
    await assistantStore.send(text, routePath);
    await scrollToBottom();
  }

  function handleKeyDown(event: KeyboardEvent) {
    // Global Cmd+K / Ctrl+K shortcut to toggle drawer
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      assistantStore.toggle();
      if (assistantStore.isOpen) {
        tick().then(() => inputEl?.focus());
      }
      return;
    }

    if (event.key === 'Escape' && assistantStore.isOpen) {
      event.preventDefault();
      assistantStore.closeDrawer();
    }
  }

  function handleInputKeyDown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      void handleSend();
    }
  }

  function sendQuickPrompt(promptText: string) {
    inputPrompt = promptText;
    void handleSend();
  }

  $effect(() => {
    // When messages change, auto-scroll to bottom
    if (assistantStore.messages.length > 0) {
      void scrollToBottom();
    }
  });

  onMount(() => {
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  });
</script>

<!-- Floating Assistant Trigger Pill (Visible when closed) -->
{#if !assistantStore.isOpen}
  <button
    class="assistant-trigger-fab"
    onclick={() => assistantStore.openDrawer()}
    aria-label="Open Axon Assistant (Cmd+K)"
    title="Axon Assistant (Cmd+K)"
  >
    <div class="fab-inner">
      <span class="pulse-indicator"></span>
      <Icon name="sparkles" size={16} />
      <span class="fab-label">Assistant</span>
      <kbd class="shortcut-kbd">⌘K</kbd>
    </div>
  </button>
{/if}

<!-- Assistant Drawer Overlay / Sheet -->
{#if assistantStore.isOpen}
  <!-- Scrim / Backdrop -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="assistant-scrim" onclick={() => assistantStore.closeDrawer()}></div>

  <aside
    bind:this={drawerEl}
    class="assistant-drawer"
    class:minimized={assistantStore.isMinimized}
    style={touchDeltaY > 0 ? `transform: translateY(${touchDeltaY}px); transition: none;` : ''}
    aria-label="Axon Assistant"
  >
    <!-- Mobile Drag Handle Bar -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="mobile-drag-bar-wrap"
      ontouchstart={handleTouchStart}
      ontouchmove={handleTouchMove}
      ontouchend={handleTouchEnd}
    >
      <div class="mobile-drag-bar"></div>
    </div>

    <!-- Header -->
    <header class="drawer-header">
      <div class="header-left">
        <div class="assistant-title">
          <Icon name="sparkles" size={16} />
          <span>Axon Assistant</span>
        </div>
        <div class="context-pill" title={currentContext.contextSummary}>
          <span class="context-dot"></span>
          <span class="context-text">{currentContext.label}</span>
        </div>
      </div>

      <div class="header-actions">
        <button
          class="btn-icon"
          onclick={() => assistantStore.clearHistory()}
          aria-label="Clear conversation"
          title="Clear history"
        >
          <Icon name="refresh" size={14} />
        </button>
        <button
          class="btn-icon"
          onclick={() => assistantStore.toggleMinimize()}
          aria-label={assistantStore.isMinimized ? 'Expand drawer' : 'Minimize drawer'}
          title={assistantStore.isMinimized ? 'Expand' : 'Minimize'}
        >
          <Icon name={assistantStore.isMinimized ? 'plus' : 'square'} size={12} />
        </button>
        <button
          class="btn-icon close-btn"
          onclick={() => assistantStore.closeDrawer()}
          aria-label="Close assistant"
          title="Close (Esc)"
        >
          <Icon name="close" size={16} />
        </button>
      </div>
    </header>

    {#if !assistantStore.isMinimized}
      <!-- Chat Body -->
      <div class="drawer-body" bind:this={chatBodyEl}>
        {#each assistantStore.messages as msg (msg.id)}
          <div class="message-row" class:user={msg.role === 'user'} class:system={msg.role === 'system'}>
            {#if msg.role === 'assistant' && msg.routing}
              <div class="msg-meta-bar">
                <span class="route-tag" title={msg.routing.reason}>{msg.routing.domain}</span>
              </div>
            {/if}

            <div class="bubble">
              <p class="bubble-text">{msg.content}</p>

              <!-- Cards carry capability data only; every write waits for a tap. -->
              {#if msg.cards && msg.cards.length > 0}
                <div class="cards-stream">
                  {#each msg.cards as card}
                    {#if card.type === 'journey_option'}
                      <JourneyActionCard card={card.data} onApply={(c, plan) => assistantStore.pinJourney(c, plan)} />
                    {:else if card.type === 'calendar_slot'}
                      <CalendarActionCard card={card.data} onApply={(c) => assistantStore.acceptCalendar(c)} />
                    {/if}
                  {/each}
                </div>
              {/if}
            </div>

            <span class="msg-time mono">
              {new Date(msg.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
            </span>
          </div>
        {/each}

        {#if assistantStore.loading}
          <div class="loading-row">
            <span class="spinner-dot"></span>
            <span class="spinner-dot"></span>
            <span class="spinner-dot"></span>
            <span class="loading-label">Asking the capabilities…</span>
          </div>
        {/if}
      </div>

      <!-- Quick Suggestion Chips based on Context -->
      {#if currentContext.quickPrompts.length > 0}
        <div class="quick-chips">
          {#each currentContext.quickPrompts as qp}
            <button class="chip-btn" onclick={() => sendQuickPrompt(qp)}>
              {qp}
            </button>
          {/each}
        </div>
      {/if}

      <!-- Footer / Input Box -->
      <footer class="drawer-footer">
        <div class="input-container">
          <textarea
            bind:this={inputEl}
            bind:value={inputPrompt}
            onkeydown={handleInputKeyDown}
            placeholder={`Ask Axon Assistant on ${currentContext.label}... (Enter to send)`}
            rows={1}
            aria-label="Message Axon Assistant"
          ></textarea>

          <button
            class="send-btn"
            disabled={!inputPrompt.trim() || assistantStore.loading}
            onclick={handleSend}
            aria-label="Send message"
          >
            <Icon name="send" size={15} />
          </button>
        </div>
      </footer>
    {/if}
  </aside>
{/if}

<style>
  /* Floating Action Button (FAB) */
  .assistant-trigger-fab {
    position: fixed;
    right: 1.5rem;
    bottom: calc(var(--soundscape-dock-height, 0px) + 1.25rem);
    z-index: 60;
    background: var(--card-bg, #18181b);
    color: var(--text-primary, #f4f4f5);
    border: 1px solid var(--card-border, #3f3f46);
    border-radius: 9999px;
    padding: 0.45rem 0.85rem;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
    cursor: pointer;
    transition:
      transform 0.15s ease,
      box-shadow 0.15s ease,
      border-color 0.15s ease;
  }

  .assistant-trigger-fab:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
    border-color: var(--primary, #06b6d4);
  }

  .fab-inner {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
  }

  .pulse-indicator {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--primary, #06b6d4);
    box-shadow: 0 0 6px var(--primary, #06b6d4);
  }

  .shortcut-kbd {
    background: var(--page-bg, #09090b);
    border: 1px solid var(--card-border, #3f3f46);
    border-radius: 4px;
    padding: 0.05rem 0.35rem;
    font-size: 0.65rem;
    font-family: inherit;
    color: var(--text-secondary, #a1a1aa);
  }

  /* Scrim */
  .assistant-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
    z-index: 80;
  }

  /* Drawer / Bottom Sheet */
  .assistant-drawer {
    position: fixed;
    top: 0;
    right: 0;
    width: clamp(380px, 32vw, 540px);
    height: 100vh;
    background: var(--page-bg, #09090b);
    border-left: 1px solid var(--card-border, #27272a);
    box-shadow: -8px 0 28px rgba(0, 0, 0, 0.4);
    z-index: 90;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .assistant-drawer.minimized {
    height: auto;
    top: auto;
    bottom: 0;
    border-top: 1px solid var(--card-border, #27272a);
    border-radius: var(--radius-lg, 12px) var(--radius-lg, 12px) 0 0;
  }

  /* Header */
  .drawer-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.85rem 1rem;
    background: var(--card-bg, #121215);
    border-bottom: 1px solid var(--card-border, #27272a);
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .assistant-title {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    color: var(--primary, #06b6d4);
  }

  .context-pill {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--primary-soft, rgba(6, 182, 212, 0.1));
    border: 1px solid rgba(6, 182, 212, 0.25);
    padding: 0.15rem 0.5rem;
    border-radius: 9999px;
    font-size: 0.65rem;
    font-weight: 500;
    color: var(--text-secondary, #d4d4d8);
    cursor: default;
  }

  .context-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--primary, #06b6d4);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .btn-icon {
    background: transparent;
    border: none;
    color: var(--text-tertiary, #71717a);
    padding: 0.35rem;
    border-radius: var(--radius-sm, 4px);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition:
      color 0.15s ease,
      background 0.15s ease;
  }

  .btn-icon:hover {
    color: var(--text-primary, #fff);
    background: var(--card-bg, #27272a);
  }

  .close-btn:hover {
    color: #ef4444;
  }

  /* Drawer Body */
  .drawer-body {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .message-row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    max-width: 90%;
  }

  .message-row.user {
    align-self: flex-end;
    align-items: flex-end;
  }

  .message-row.system {
    align-self: center;
    align-items: center;
    max-width: 100%;
  }

  .msg-meta-bar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-bottom: 0.25rem;
    font-size: 0.62rem;
  }

  .route-tag {
    color: var(--text-tertiary, #a1a1aa);
    padding: 0.05rem 0.35rem;
    border-radius: 4px;
    border: 1px solid var(--card-border, #27272a);
  }

  .bubble {
    background: var(--card-bg, #18181b);
    border: 1px solid var(--card-border, #27272a);
    border-radius: var(--radius-md, 8px);
    padding: 0.65rem 0.85rem;
    color: var(--text-primary, #f4f4f5);
    font-size: var(--text-xs, 0.75rem);
    line-height: 1.5;
    word-break: break-word;
  }

  .message-row.user .bubble {
    background: var(--primary-soft, rgba(6, 182, 212, 0.15));
    border-color: rgba(6, 182, 212, 0.3);
    color: var(--text-primary, #ffffff);
  }

  .message-row.system .bubble {
    background: transparent;
    border: 1px dashed var(--card-border, #3f3f46);
    color: var(--text-tertiary, #a1a1aa);
    font-size: 0.7rem;
    text-align: center;
  }

  .bubble-text {
    margin: 0;
    white-space: pre-wrap;
  }

  .cards-stream {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .msg-time {
    font-size: 0.6rem;
    color: var(--text-tertiary, #71717a);
    margin-top: 0.2rem;
  }

  .loading-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.7rem;
    color: var(--text-tertiary, #a1a1aa);
    padding: 0.5rem 0;
  }

  .spinner-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--primary, #06b6d4);
    animation: bounce 1.2s infinite ease-in-out;
  }

  .spinner-dot:nth-child(2) {
    animation-delay: 0.2s;
  }

  .spinner-dot:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes bounce {
    0%, 80%, 100% {
      transform: scale(0);
      opacity: 0.3;
    }
    40% {
      transform: scale(1);
      opacity: 1;
    }
  }

  /* Quick Suggestion Chips */
  .quick-chips {
    display: flex;
    gap: 0.4rem;
    overflow-x: auto;
    padding: 0.5rem 1rem;
    background: var(--page-bg, #09090b);
    border-top: 1px solid var(--card-border, #27272a);
    scrollbar-width: none;
  }

  .quick-chips::-webkit-scrollbar {
    display: none;
  }

  .chip-btn {
    white-space: nowrap;
    background: var(--card-bg, #18181b);
    border: 1px solid var(--card-border, #27272a);
    border-radius: 9999px;
    padding: 0.25rem 0.65rem;
    font-size: 0.68rem;
    color: var(--text-secondary, #d4d4d8);
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      color 0.15s ease;
  }

  .chip-btn:hover {
    border-color: var(--primary, #06b6d4);
    color: var(--primary, #06b6d4);
  }

  /* Input Footer */
  .drawer-footer {
    padding: 0.75rem 1rem;
    background: var(--card-bg, #121215);
    border-top: 1px solid var(--card-border, #27272a);
  }

  .input-container {
    display: flex;
    align-items: center;
    background: var(--page-bg, #09090b);
    border: 1px solid var(--card-border, #3f3f46);
    border-radius: var(--radius-md, 8px);
    padding: 0.4rem 0.6rem;
    gap: 0.5rem;
  }

  .input-container:focus-within {
    border-color: var(--primary, #06b6d4);
  }

  textarea {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text-primary, #fff);
    font-family: inherit;
    font-size: var(--text-xs, 0.75rem);
    resize: none;
    line-height: 1.4;
    max-height: 120px;
  }

  .send-btn {
    background: var(--primary, #06b6d4);
    color: var(--text-inverse, #000);
    border: none;
    border-radius: var(--radius-sm, 4px);
    padding: 0.35rem 0.5rem;
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: opacity 0.15s ease;
  }

  .send-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Drag handle on mobile */
  .mobile-drag-bar-wrap {
    display: none;
    width: 100%;
    padding: 0.5rem 0 0.25rem;
    cursor: grab;
    touch-action: pan-y;
    justify-content: center;
    align-items: center;
  }

  .mobile-drag-bar {
    width: 38px;
    height: 4px;
    border-radius: 9999px;
    background: var(--card-border, #3f3f46);
  }

  /* Responsive Mobile Bottom Sheet */
  @media (width < 48rem) {
    .mobile-drag-bar-wrap {
      display: flex;
    }

    .assistant-drawer {
      top: auto;
      bottom: 0;
      width: 100%;
      height: min(88dvh, calc(100dvh - env(safe-area-inset-top, 24px)));
      border-left: none;
      border-top: 1px solid var(--card-border, #27272a);
      border-radius: var(--radius-lg, 16px) var(--radius-lg, 16px) 0 0;
      transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
      overscroll-behavior: contain;
    }

    .drawer-footer {
      padding-bottom: max(0.85rem, env(safe-area-inset-bottom, 16px));
    }

    .assistant-trigger-fab {
      right: 1rem;
      bottom: calc(var(--soundscape-dock-height, 0px) + max(1rem, env(safe-area-inset-bottom, 12px)));
    }
  }

  @media (width < 38rem) {
    .assistant-trigger-fab {
      display: none;
    }
  }
</style>
