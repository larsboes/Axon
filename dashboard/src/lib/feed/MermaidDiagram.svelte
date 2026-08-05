<script lang="ts">
  /**
   * One Mermaid diagram, rendered from source the server already validated.
   *
   * Mermaid is loaded by dynamic import inside this component and nowhere else,
   * so its bundle is code-split onto the reader route rather than shipped to
   * every page — the same shape the Trips workspace uses for maplibre-gl
   * (dashboard/README.md). `startOnLoad` stays off: the reader decides when a
   * diagram appears, and a library that scans the document on load would also
   * try to render the code fences inside an article body.
   *
   * The server's `extract_mermaid` gate means an unrenderable string never
   * reaches this component. A parse error here is therefore a real one — a
   * diagram Mermaid's own parser rejects — and is shown rather than swallowed,
   * with the source kept visible so the failure is inspectable.
   */
  import { onMount } from "svelte";

  let { source, theme = "auto" }: { source: string; theme?: "auto" | "dark" | "light" } = $props();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let container = $state<HTMLDivElement | null>(null);
  let sequence = 0;

  /** Mermaid's themes are chosen at init, so the page's own dark class decides. */
  function resolvedTheme(): "dark" | "default" {
    if (theme === "dark") return "dark";
    if (theme === "light") return "default";
    return document.documentElement.classList.contains("dark") ? "dark" : "default";
  }

  async function draw(diagram: string) {
    const run = ++sequence;
    error = null;
    try {
      const mermaid = (await import("mermaid")).default;
      mermaid.initialize({
        startOnLoad: false,
        theme: resolvedTheme(),
        securityLevel: "strict",
        fontFamily: "var(--font-sans)",
      });
      // A fresh id per render: Mermaid keys its internal definitions by id and
      // reuses a stale one otherwise, which shows the previous diagram after a
      // regenerate.
      const { svg: rendered } = await mermaid.render(`axon-mermaid-${run}`, diagram);
      // A later press already superseded this one.
      if (run !== sequence) return;
      svg = rendered;
    } catch (cause) {
      if (run !== sequence) return;
      svg = null;
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  onMount(() => {
    if (source) void draw(source);
  });

  $effect(() => {
    if (container && source) void draw(source);
  });
</script>

<div class="diagram" bind:this={container}>
  {#if svg}
    <!-- Mermaid returns an SVG string; there is no element API to bind to. The
         input is the server-validated diagram source, not operator HTML. -->
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    {@html svg}
  {:else if error}
    <p class="diagram-error">Mermaid could not draw this: {error}</p>
  {:else}
    <p class="diagram-pending">Drawing…</p>
  {/if}
  {#if error}
    <pre class="diagram-source">{source}</pre>
  {/if}
</div>

<style>
  .diagram {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
    /* A wide flowchart scrolls inside its own box rather than widening the
       reader column. */
    overflow-x: auto;
  }

  .diagram :global(svg) {
    max-width: 100%;
    height: auto;
  }

  .diagram-error {
    margin: 0;
    color: var(--danger);
    font-size: 0.875rem;
  }

  .diagram-pending {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.875rem;
  }

  .diagram-source {
    margin: 0;
    padding: 0.75rem;
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: 0.75rem;
    overflow-x: auto;
  }
</style>
