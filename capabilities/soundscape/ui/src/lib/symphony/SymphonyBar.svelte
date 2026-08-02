<script lang="ts">
  import { onDestroy } from "svelte";
  import { symphonyStore } from "./symphonyStore.svelte";

  let { onpeek, onexpand }: { onpeek?: () => void; onexpand?: () => void } = $props();

  // Read through the store rather than the engine: in this mode there may be no
  // engine in this tab at all, only a view of what another surface is playing.
  const playing = $derived(symphonyStore.isPlaying);
  const contested = $derived(symphonyStore.contested);
  const elsewhere = $derived(
    symphonyStore.isPlaying && symphonyStore.host !== null && !symphonyStore.isHost,
  );

  let peekTimer: ReturnType<typeof setTimeout> | null = null;

  function cancelPeek() {
    if (peekTimer === null) return;
    clearTimeout(peekTimer);
    peekTimer = null;
  }

  function queuePeek() {
    if (!onpeek || !matchMedia("(hover: hover) and (pointer: fine)").matches) return;
    cancelPeek();
    peekTimer = setTimeout(() => {
      peekTimer = null;
      onpeek();
    }, 220);
  }

  function openPeek() {
    cancelPeek();
    onpeek?.();
  }

  onDestroy(cancelPeek);
</script>

<div class="bar">
  <button
    class="play"
    onclick={() => symphonyStore.togglePlay()}
    aria-label={playing ? "Pause" : "Play"}
    title={playing ? "Pause" : "Play"}
  >
    {#if playing}
      <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1" /><rect x="14" y="5" width="4" height="14" rx="1" /></svg>
    {:else}
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5.2v13.6a1 1 0 0 0 1.54.84l10.2-6.8a1 1 0 0 0 0-1.68L9.54 4.36A1 1 0 0 0 8 5.2Z" /></svg>
    {/if}
  </button>

  <div class="what">
    <button
      class="peek-trigger"
      onpointerenter={queuePeek}
      onpointerleave={cancelPeek}
      onclick={openPeek}
      aria-label="Open Soundscape controls"
      title="Open Soundscape controls"
    >
      <span class="np">{symphonyStore.nowPlayingText}</span>
      <span class="peek-hint" aria-hidden="true">↑</span>
    </button>
    {#if contested}
      <span class="note contested">
        playing in {contested.label}
        <button class="link" onclick={() => symphonyStore.takeOver()}>take over here</button>
        <button class="link muted" onclick={() => symphonyStore.dismissContest()}>cancel</button>
      </span>
    {:else if elsewhere}
      <!-- Says where the sound is instead of pretending this surface makes it. -->
      <span class="note">Audio is playing in {symphonyStore.host?.label}</span>
    {:else if !symphonyStore.connected}
      <span class="note warn">Conductor unavailable — nothing is being synchronised</span>
    {/if}
  </div>

  <input
    class="vol"
    type="range"
    min="0"
    max="100"
    aria-label="Volume"
    value={symphonyStore.volume * 100}
    oninput={(e) => symphonyStore.setVolume(+(e.currentTarget as HTMLInputElement).value / 100)}
  />

  {#if onexpand}
    <button class="expand" onclick={onexpand} aria-label="Full screen" title="Full screen">
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 10V4h6M20 14v6h-6M20 10V4h-6M4 14v6h6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
      </svg>
    </button>
  {/if}
</div>

<style>
  /* Deliberately its own look rather than the dashboard's tokens: this lives in a
     different origin, and piping a theme across that boundary buys less than a
     player that simply looks like a player. */
  .bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    height: 100%;
    padding: 0 0.85rem;
    background: #0d0b16;
    border-top: 1px solid #241f38;
    color: #e6e2f5;
    font: 500 0.8rem/1.2 ui-sans-serif, system-ui, sans-serif;
  }

  button {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font: inherit;
  }

  .play {
    flex: none;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: #a78bfa;
    color: #17102b;
  }

  .play:hover {
    background: #c4b5fd;
  }

  .play svg {
    width: 17px;
    height: 17px;
    fill: currentColor;
  }

  .what {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }

  .peek-trigger {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.28rem 0.38rem;
    margin: -0.28rem -0.38rem;
    border-radius: 5px;
    text-align: left;
  }

  .peek-trigger:hover,
  .peek-trigger:focus-visible {
    background: #181622;
    outline: none;
  }

  .np {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    letter-spacing: 0.02em;
  }

  .peek-hint {
    flex: none;
    color: #827c9d;
    font-size: 0.72rem;
  }

  .note {
    font-size: 0.68rem;
    color: #9d94c4;
  }

  .note.contested {
    color: #fbbf24;
  }

  .note.warn {
    color: #f87171;
  }

  .link {
    padding: 0;
    margin-left: 0.4rem;
    color: #a78bfa;
    text-decoration: underline;
  }

  .link.muted {
    color: #9d94c4;
  }

  .vol {
    flex: none;
    width: 86px;
    accent-color: #a78bfa;
  }

  .expand {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    color: #9d94c4;
  }

  .expand:hover {
    background: #1b1730;
    color: #e6e2f5;
  }

  .expand svg {
    width: 15px;
    height: 15px;
  }

  @media (max-width: 640px) {
    .vol {
      width: 64px;
    }
  }
</style>
