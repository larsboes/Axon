<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import type { PresetKey } from "./types";
  import { QUICK_SCENARIOS } from "./scenarios";
  import { symphonyStore } from "./symphonyStore.svelte";

  let { oncollapse, onexpand }: { oncollapse: () => void; onexpand: () => void } = $props();

  const intents: Array<{ key: PresetKey; label: string }> = [
    { key: "focus", label: "Focus" },
    { key: "relax", label: "Relax" },
    { key: "sleep", label: "Sleep" },
  ];

  const hour = new Date().getHours();
  const recommended = hour < 6 || hour >= 22
    ? intents[2]
    : hour >= 18
      ? intents[1]
      : intents[0];

  let bands = $state([0.24, 0.42, 0.66, 0.38, 0.74, 0.48, 0.31, 0.57, 0.28]);
  let signalTimer: ReturnType<typeof setInterval> | null = null;
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

  function cancelClose() {
    if (closeTimer === null) return;
    clearTimeout(closeTimer);
    closeTimer = null;
  }

  function queueClose() {
    cancelClose();
    closeTimer = setTimeout(oncollapse, 320);
  }

  function readSignal() {
    const audio = symphonyStore.getAudioEngine();
    if (!audio.analyser || !audio.freqData || !audio.playing) {
      bands = bands.map((_, index) => 0.2 + 0.08 * ((index * 5) % 4));
      return;
    }
    audio.analyser.getByteFrequencyData(audio.freqData as never);
    const stride = Math.max(1, Math.floor(48 / bands.length));
    bands = bands.map((_, index) => {
      const start = index * stride;
      let sum = 0;
      for (let offset = 0; offset < stride; offset++) sum += audio.freqData?.[start + offset] ?? 0;
      return Math.max(0.12, sum / (stride * 255));
    });
  }

  onMount(() => {
    signalTimer = setInterval(readSignal, 110);
  });

  onDestroy(() => {
    if (signalTimer !== null) clearInterval(signalTimer);
    cancelClose();
  });
</script>

<section
  class="peek"
  aria-label="Soundscape controls"
  onpointerenter={cancelClose}
  onpointerleave={queueClose}
  onfocusin={cancelClose}
  onfocusout={(event) => {
    if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node | null)) queueClose();
  }}
>
  <div class="visual" aria-hidden="true">
    <div class="signal">
      {#each bands as band}
        <span style={`transform: scaleY(${band})`}></span>
      {/each}
    </div>
    <div class="visual-copy">
      <span>Soundscape</span>
      <strong>{symphonyStore.nowPlayingText}</strong>
      <small>{symphonyStore.isPlaying ? "playing on this device" : "ready"}</small>
    </div>
  </div>

  <div class="panel">
    <header>
      <div>
        <p class="eyebrow">
          What do you need sound for?
          <button
            class="recommendation"
            onclick={() => symphonyStore.setPreset(recommended.key)}
            title="Based only on local time; does not start audio"
          >
            Fits now: {recommended.label}
          </button>
        </p>
        <div class="intents" aria-label="Sound intent">
          {#each intents as intent}
            <button
              class:active={symphonyStore.preset === intent.key}
              onclick={() => symphonyStore.setPreset(intent.key)}
            >
              {intent.label}
            </button>
          {/each}
        </div>
      </div>
      <div class="window-actions">
        <button class="icon" onclick={oncollapse} aria-label="Back to bar" title="Back to bar">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
        </button>
        <button class="icon" onclick={onexpand} aria-label="Full screen" title="Full screen">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 10V4h6M20 14v6h-6M20 10V4h-6M4 14v6h6" /></svg>
        </button>
      </div>
    </header>

    <div class="session-line" aria-live="polite">
      {#if symphonyStore.session}
        <strong>{symphonyStore.sessionLabel}</strong>
        <span>{symphonyStore.sessionClock} remaining</span>
        <button onclick={() => symphonyStore.cancelSession()}>End timer</button>
      {:else}
        <strong>Quick start</strong>
        <span>A clearly bounded session, then it goes quiet.</span>
      {/if}
    </div>

    <div class="scenarios">
      {#each QUICK_SCENARIOS as scenario}
        <button
          class:active={symphonyStore.session?.scenario === scenario.key}
          onclick={() => void symphonyStore.startScenario(scenario)}
          aria-label={`Start ${scenario.label} for ${scenario.minutes} minutes`}
        >
          <span>
            <strong>{scenario.label}</strong>
            <small>{scenario.detail}</small>
          </span>
          <b>{scenario.minutes}</b>
        </button>
      {/each}
    </div>

    <div class="transport">
      <button
        class="play"
        onclick={() => void symphonyStore.togglePlay()}
        aria-label={symphonyStore.isPlaying ? "Pause" : "Play"}
      >
        {#if symphonyStore.isPlaying}
          <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1" /><rect x="14" y="5" width="4" height="14" rx="1" /></svg>
        {:else}
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5.2v13.6a1 1 0 0 0 1.54.84l10.2-6.8a1 1 0 0 0 0-1.68L9.54 4.36A1 1 0 0 0 8 5.2Z" /></svg>
        {/if}
      </button>

      <label>
        <span>Intensity</span>
        <input
          type="range"
          min="0"
          max="100"
          value={symphonyStore.energy * 100}
          oninput={(event) => symphonyStore.setEnergy(+(event.currentTarget as HTMLInputElement).value / 100)}
        />
      </label>
      <label>
        <span>Volume</span>
        <input
          type="range"
          min="0"
          max="100"
          value={symphonyStore.volume * 100}
          oninput={(event) => symphonyStore.setVolume(+(event.currentTarget as HTMLInputElement).value / 100)}
        />
      </label>

      {#if symphonyStore.contested}
        <div class="takeover">
          <span>Audio is playing in {symphonyStore.contested.label}</span>
          <button onclick={() => void symphonyStore.takeOver()}>Take over here</button>
        </div>
      {/if}
    </div>
  </div>
</section>

<style>
  .peek {
    --ink: #efedf4;
    --muted: #918c9e;
    --line: #2b2930;
    --violet: #a892dd;
    --cyan: #75aeb4;
    display: grid;
    grid-template-columns: minmax(270px, 0.72fr) minmax(560px, 1.45fr);
    height: 100%;
    border-top: 1px solid var(--line);
    background: #0a0a0c;
    color: var(--ink);
    font: 500 0.82rem/1.25 -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif;
  }

  button,
  input {
    font: inherit;
  }

  button {
    border: 0;
    color: inherit;
    cursor: pointer;
  }

  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--cyan);
    outline-offset: 2px;
  }

  .visual {
    position: relative;
    overflow: hidden;
    border-right: 1px solid var(--line);
    background: #101015;
  }

  .visual::before,
  .visual::after {
    content: "";
    position: absolute;
    inset: 18% 12%;
    border: 1px solid #292633;
    transform: rotate(-8deg);
  }

  .visual::after {
    inset: 28% 20%;
    border-color: #263337;
    transform: rotate(11deg);
  }

  .signal {
    position: absolute;
    inset: 22px 26px 84px;
    display: flex;
    align-items: stretch;
    justify-content: center;
    gap: clamp(5px, 1.1vw, 14px);
  }

  .signal span {
    width: clamp(4px, 0.62vw, 9px);
    min-height: 100%;
    transform-origin: center;
    background: var(--violet);
    transition: transform 0.1s linear;
  }

  .signal span:nth-child(3n + 2) {
    background: var(--cyan);
  }

  .visual-copy {
    position: absolute;
    left: 28px;
    right: 28px;
    bottom: 24px;
    display: grid;
    gap: 3px;
  }

  .visual-copy span,
  .eyebrow {
    margin: 0;
    color: var(--muted);
    font-size: 0.66rem;
    letter-spacing: 0.14em;
    text-transform: uppercase;
  }

  .recommendation {
    margin-left: 8px;
    padding: 0;
    background: transparent;
    color: var(--cyan);
    font-size: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
  }

  .recommendation:hover {
    color: var(--ink);
  }

  .visual-copy strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 1rem;
  }

  .visual-copy small {
    color: var(--cyan);
  }

  .panel {
    min-width: 0;
    display: grid;
    grid-template-rows: auto auto 1fr auto;
    padding: 20px 24px 18px;
  }

  header,
  .session-line,
  .transport {
    display: flex;
    align-items: center;
  }

  header {
    justify-content: space-between;
    gap: 18px;
  }

  .intents {
    display: flex;
    gap: 20px;
    margin-top: 8px;
  }

  .intents button {
    padding: 0 0 5px;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--muted);
    font-size: 0.95rem;
  }

  .intents button:hover,
  .intents button.active {
    border-bottom-color: var(--cyan);
    color: var(--ink);
  }

  .window-actions {
    display: flex;
    gap: 4px;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 5px;
    background: transparent;
    color: var(--muted);
  }

  .icon:hover {
    background: #18171c;
    color: var(--ink);
  }

  .icon svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .session-line {
    gap: 12px;
    min-height: 42px;
    margin-top: 8px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
  }

  .session-line strong {
    color: var(--ink);
  }

  .session-line button {
    margin-left: auto;
    padding: 0;
    background: transparent;
    color: var(--cyan);
    font-size: 0.72rem;
  }

  .scenarios {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    align-content: center;
    column-gap: 22px;
  }

  .scenarios > button {
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 10px 3px;
    border-bottom: 1px solid var(--line);
    background: transparent;
    text-align: left;
  }

  .scenarios > button:hover,
  .scenarios > button.active {
    border-bottom-color: var(--violet);
  }

  .scenarios span {
    min-width: 0;
    display: grid;
    gap: 2px;
  }

  .scenarios strong,
  .scenarios small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scenarios small {
    color: var(--muted);
    font-size: 0.7rem;
  }

  .scenarios b {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--line);
    border-radius: 50%;
    color: var(--muted);
    font-size: 0.7rem;
  }

  .scenarios button.active b {
    border-color: var(--violet);
    color: var(--ink);
  }

  .transport {
    gap: 18px;
    padding-top: 13px;
  }

  .play {
    flex: none;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: var(--violet);
    color: #121016;
  }

  .play svg {
    width: 18px;
    height: 18px;
    fill: currentColor;
  }

  .transport label {
    display: grid;
    gap: 5px;
    color: var(--muted);
    font-size: 0.68rem;
  }

  input[type="range"] {
    width: clamp(90px, 10vw, 150px);
    accent-color: var(--cyan);
  }

  .takeover {
    min-width: 0;
    margin-left: auto;
    display: grid;
    justify-items: end;
    color: #d7b96f;
    font-size: 0.68rem;
  }

  .takeover button {
    padding: 0;
    background: transparent;
    color: var(--violet);
  }

  @media (max-width: 880px) {
    .peek {
      grid-template-columns: 1fr;
    }

    .visual {
      display: none;
    }
  }

  @media (max-width: 600px) {
    .panel {
      padding-inline: 16px;
    }

    .session-line span,
    .scenarios small,
    .transport label:first-of-type {
      display: none;
    }

    .intents {
      gap: 13px;
    }

    .transport {
      gap: 12px;
    }

    input[type="range"] {
      width: 92px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .signal span {
      transition: none;
    }
  }
</style>
