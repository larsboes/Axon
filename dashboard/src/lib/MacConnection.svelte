<script lang="ts">
  // The app's canonical Axon node setting. Rendered only inside the Tauri app: the web shell
  // reaches the Mac through its own proxy and has nothing to set.
  import { onMount } from "svelte";
  import Overlay from "./Overlay.svelte";
  import {
    bridgeErrorText,
    getConnectionSettings,
    HEALTH_PATH,
    inTauri,
    macRequest,
    setCanonicalBaseUrl,
  } from "./mac-bridge";

  const shown = inTauri();
  let open = $state(false);
  let saved = $state<string | null>(null);
  let draft = $state("");
  let busy = $state(false);
  let result = $state<{ ok: boolean; text: string } | null>(null);

  onMount(async () => {
    if (!shown) return;
    try {
      saved = (await getConnectionSettings()).canonical_base_url;
      draft = saved ?? "";
    } catch (error) {
      result = { ok: false, text: bridgeErrorText(error) };
    }
  });

  async function save() {
    busy = true;
    result = null;
    try {
      saved = (await setCanonicalBaseUrl(draft)).canonical_base_url;
      draft = saved ?? "";
      result = { ok: true, text: saved ? "Saved." : "Cleared." };
    } catch (error) {
      result = { ok: false, text: bridgeErrorText(error) };
    } finally {
      busy = false;
    }
  }

  async function test() {
    busy = true;
    result = null;
    try {
      const answer = await macRequest(HEALTH_PATH);
      const ok = answer.status >= 200 && answer.status < 300;
      result = { ok, text: `HTTP ${answer.status}: ${answer.body.slice(0, 200)}` };
    } catch (error) {
      result = { ok: false, text: bridgeErrorText(error) };
    } finally {
      busy = false;
    }
  }
</script>

{#if shown}
  <button class="link" type="button" onclick={() => (open = true)}>
    Axon connection{saved ? "" : " (not set)"}
  </button>
  {#if open}
    <Overlay title="Axon connection" onClose={() => (open = false)} {busy}>
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label>
          Canonical node address
          <input
            type="url"
            bind:value={draft}
            placeholder="https://<name>.ts.net"
            autocapitalize="off"
            autocomplete="off"
            spellcheck="false"
          />
        </label>
        <p class="hint">
          The HTTPS address of the canonical Axon node. The app keeps it on this device only. The Mac is the canonical node for now; a home server can replace it later.
        </p>
        <div class="actions">
          <button type="submit" disabled={busy}>Save</button>
          <button type="button" disabled={busy || !saved} onclick={() => void test()}>Test</button>
        </div>
        {#if result}
          <p class="result" class:bad={!result.ok}>{result.text}</p>
        {/if}
      </form>
    </Overlay>
  {/if}
{/if}

<style>
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  label {
    display: grid;
    gap: 0.35rem;
  }
  input {
    font: inherit;
    padding: 0.4rem 0.5rem;
  }
  .hint {
    opacity: 0.75;
    font-size: var(--text-sm);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  .result {
    word-break: break-word;
  }
  .bad {
    color: var(--danger, #b00020);
  }
</style>
