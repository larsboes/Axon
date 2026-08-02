<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, hasPanel, type CapabilityView } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";

  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);

  onMount(() => capabilities.subscribe(5_000));

  async function act(name: string, action: "start" | "stop"): Promise<void> {
    busy = name;
    error = null;
    try {
      await axonStatus[action](name);
      await capabilities.refresh();
    } catch (e) {
      error = `${action} ${name}: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      busy = null;
    }
  }

  // Three states, not two. A container with no declared health surface is unknown from
  // here, and painting that as "stopped" would be a lie with a colour attached.
  function state_(c: CapabilityView): { dot: string; label: string } {
    if (c.up === null) return { dot: "unknown", label: "unknown" };
    return c.up ? { dot: "up", label: "running" } : { dot: "down", label: "stopped" };
  }
</script>

<PageHeader
  badge="Machine"
  title="Capabilities"
  desc="What this machine has enabled, what is running, and what starts on demand."
/>

{#if capabilities.offline}
  <p class="offline">
    <Icon name="wifi-off" />
    axon-status is unavailable. Start it with:
    <code>tools/service-runner.sh start axon-status</code>
  </p>
{:else}
  {#if error}
    <p class="error">{error}</p>
  {/if}

  <ul>
    {#each capabilities.items as c (c.name)}
      {@const s = state_(c)}
      <li class="card">
        <div class="left">
          <span class="dot {s.dot}"></span>
          <div class="ident">
            <p class="name">
              {c.name}
              {#if c.scope === "spine"}<span class="tag mono">spine</span>{/if}
              {#if c.autostart === "true"}<span class="tag mono">autostart</span>{/if}
            </p>
            <p class="sub mono">
              {c.kind}{c.port ? ` · :${c.port}` : ""} · {s.label}
            </p>
          </div>
        </div>

        <div class="actions">
          {#if hasPanel(c) && c.up}
            <a class="btn" href="/projects">Project <Icon name="arrow-right" size={13} /></a>
          {/if}
          {#if c.up !== null}
            {#if busy === c.name}
              <span class="btn"><Icon name="loader" size={14} /></span>
            {:else if c.up}
              <button class="btn" onclick={() => act(c.name, "stop")}>
                <Icon name="square" size={12} /> Stop
              </button>
            {:else}
              <button class="btn" onclick={() => act(c.name, "start")}>
                <Icon name="play" size={12} /> Start
              </button>
            {/if}
          {/if}
        </div>
      </li>
    {/each}
  </ul>

  <p class="note">
    Only services marked <code>autostart</code> run without being requested. Everything else
    starts when you open it.
  </p>
{/if}

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  li {
    padding: 0.75rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .left {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    min-width: 0;
  }

  .dot {
    height: 0.5rem;
    width: 0.5rem;
    border-radius: 999px;
    flex-shrink: 0;
    background-color: var(--text-tertiary);
  }

  .dot.up {
    background-color: var(--success);
  }

  .dot.down {
    background-color: var(--warning);
  }

  .ident {
    min-width: 0;
  }

  .name {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .sub {
    margin: 0.1rem 0 0;
    font-size: 0.625rem;
    color: var(--text-tertiary);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex-shrink: 0;
  }

  .offline,
  .error {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    border-radius: var(--radius-md);
    background-color: var(--warning-soft);
    color: var(--warning);
    font-size: 0.8125rem;
  }

  .note {
    margin-top: 1rem;
    font-size: 0.75rem;
    color: var(--text-tertiary);
  }
</style>
