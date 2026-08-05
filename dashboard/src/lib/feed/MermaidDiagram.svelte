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
   * Colours come from `mermaid-theme.ts` and nowhere else. Mermaid resolves its
   * theme at `initialize`, not at render, so a theme change means re-initialising
   * and drawing again; the observer below is what notices. It watches the root
   * element's class rather than taking a prop, because the layout owns the
   * toggle and does not know this component exists.
   *
   * The server's `extract_mermaid` gate means an unrenderable string never
   * reaches here. A parse error is therefore a real one and is shown rather than
   * swallowed, with the source kept visible so the failure is inspectable.
   */
  import { onMount } from "svelte";
  import { mermaidConfig } from "$lib/feed/mermaid-theme";

  let { source }: { source: string } = $props();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let dark = $state(false);
  let sequence = 0;

  async function draw(diagram: string, darkMode: boolean) {
    const run = ++sequence;
    error = null;
    try {
      const mermaid = (await import("mermaid")).default;
      mermaid.initialize(mermaidConfig(darkMode));
      // A fresh id per render: Mermaid keys its internal definitions by id and
      // reuses a stale one otherwise, which shows the previous diagram after a
      // regenerate or a theme flip.
      const { svg: rendered } = await mermaid.render(`axon-mermaid-${run}`, diagram);
      if (run !== sequence) return; // a later run already superseded this one
      svg = rendered;
    } catch (cause) {
      if (run !== sequence) return;
      svg = null;
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  onMount(() => {
    const root = document.documentElement;
    dark = root.classList.contains("dark");
    const observer = new MutationObserver(() => {
      dark = root.classList.contains("dark");
    });
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  });

  $effect(() => {
    if (source) void draw(source, dark);
  });
</script>

<figure class="diagram" class:dark>
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
</figure>

<style>
  /* The figure sits on the palette's own paper in light mode and on the page's
     card in dark, so the node fills keep the same contrast either way without a
     white plate glaring out of a dark reader. */
  .diagram {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin: 0;
    padding: 1.25rem 1rem;
    border: 1px solid #ddd6cf; /* palette root: grid */
    border-radius: var(--radius-md);
    background: #fbfaf7; /* palette root: paper */
    /* A wide flowchart scrolls inside its own box rather than widening the
       reader column. */
    overflow-x: auto;
  }

  .diagram.dark {
    border-color: var(--card-border);
    background: var(--card-bg);
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
