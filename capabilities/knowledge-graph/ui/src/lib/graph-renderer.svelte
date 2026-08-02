<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { GraphNode, GraphEdge } from './api';

  // vis-network is loaded from a CDN at runtime; its shape is declared in src/app.d.ts,
  // because `declare` is not valid inside a Svelte instance script.

  let { nodes = [], edges = [], highlight = '', onNodeClick }: {
    nodes: GraphNode[];
    edges: GraphEdge[];
    highlight?: string;
    onNodeClick?: (nodeId: string) => void;
  } = $props();

  let container: HTMLDivElement;
  let network: VisNetwork | null = null;

  function buildVisNodes(ns: GraphNode[]): any[] {
    return ns.map(n => ({
      id: n.id,
      label: n.label,
      title: `${n.label}\n${n.source_file}`,
      group: n.group,
      // Color by file type
      color: {
        background: n.file_type === 'code' ? '#2d5f8a' :
                    n.file_type === 'doc' || n.file_type === 'markdown' ? '#5a8a2d' :
                    n.file_type === 'config' || n.file_type === 'json' ? '#8a6f2d' :
                    '#4a4a6a',
        border: n.file_type === 'code' ? '#4e79a7' :
                n.file_type === 'doc' || n.file_type === 'markdown' ? '#76b74e' :
                n.file_type === 'config' || n.file_type === 'json' ? '#b79a4e' :
                '#6a6a8a',
        highlight: { background: '#f0c040', border: '#e0a000' },
      },
      borderWidth: 1,
      borderWidthSelected: 2,
      font: { color: '#e0e0e0', size: 11 },
      size: 8,
      shape: 'dot',
    }));
  }

  function buildVisEdges(es: GraphEdge[]): any[] {
    return es.map(e => ({
      from: e.from,
      to: e.to,
      title: e.label || '',
      color: { color: '#3a3a5e', hover: '#5a7aae', highlight: '#7a9ace' },
      width: 0.5,
      smooth: { type: 'continuous' },
    }));
  }

  function initNetwork() {
    if (!container || !window.vis) return;

    const visNodes = new window.vis.DataSet(buildVisNodes(nodes));
    const visEdges = new window.vis.DataSet(buildVisEdges(edges));

    // Bound to a local first, then published: the handlers below close over the instance
    // they belong to, and reading the module-level `network` inside them would race the
    // $effect that destroys and replaces it on every data change.
    const instance = new window.vis.Network(container, {
      nodes: visNodes,
      edges: visEdges,
    }, {
      physics: {
        solver: 'forceAtlas2Based',
        forceAtlas2Based: {
          gravitationalConstant: -32,
          centralGravity: 0.005,
          springLength: 160,
          springConstant: 0.02,
          damping: 0.4,
        },
        stabilization: { iterations: 200 },
      },
      layout: { improvedLayout: true },
      interaction: {
        hover: true,
        tooltipDelay: 200,
        navigationButtons: true,
        keyboard: true,
      },
      groups: useGroups(),
      edges: {
        smooth: { type: 'continuous' },
      },
      configure: { enabled: false },
    });

    instance.on('click', (params) => {
      if (params.nodes?.length && onNodeClick) {
        onNodeClick(params.nodes[0]);
      }
    });

    instance.on('doubleClick', (params) => {
      if (params.nodes?.length) {
        instance.focus(params.nodes[0], { scale: 1.5, animation: true });
      }
    });

    network = instance;
  }

  function useGroups(): Record<string, any> {
    // Assign a hue per community (cycle through 36 hues)
    const groups: Record<string, any> = {};
    const seen = new Set<string>();
    for (const n of nodes) {
      if (!seen.has(n.group)) {
        seen.add(n.group);
        const num = n.community;
        const hue = ((num * 47) % 360);
        groups[n.group] = {
          color: { background: `hsl(${hue}, 35%, 25%)`, border: `hsl(${hue}, 50%, 45%)` },
        };
      }
    }
    return groups;
  }

  function applyHighlight(term: string) {
    if (!network) return;
    if (!term) {
      network.selectNodes([]);
      return;
    }
    const matching = nodes
      .filter(n => n.label.toLowerCase().includes(term.toLowerCase()))
      .map(n => n.id);
    network.selectNodes(matching);
    if (matching.length > 0) {
      network.focus(matching[0], { scale: 1.3, animation: true });
    }
  }

  $effect(() => {
    // Rebuild on data change
    if (network && container) {
      network.destroy();
      network = null;
      initNetwork();
    }
  });

  $effect(() => {
    if (highlight && network) {
      applyHighlight(highlight);
    }
  });

  onMount(() => {
    // Load vis-network from CDN (same source graphify uses)
    if (!document.querySelector('script[src*="vis-network"]')) {
      const script = document.createElement('script');
      script.src = 'https://unpkg.com/vis-network@9.1.6/standalone/umd/vis-network.min.js';
      script.integrity = 'sha384-Ux6phic9PEHJ38YtrijhkzyJ8yQlH8i/+buBR8s3mAZOJrP1gwyvAcIYl3GWtpX1';
      script.crossOrigin = 'anonymous';
      script.onload = () => initNetwork();
      document.head.appendChild(script);
    } else if (window.vis) {
      initNetwork();
    }
  });

  onDestroy(() => {
    if (network) {
      network.destroy();
      network = null;
    }
  });
</script>

<div bind:this={container} class="graph-container"></div>

<style>
  .graph-container {
    flex: 1;
    height: 100%;
    min-height: 0;
  }
  :global(.vis-network) {
    outline: none;
  }
  :global(.vis-tooltip) {
    background: #1a1a2e !important;
    border: 1px solid #3a3a5e !important;
    color: #e0e0e0 !important;
    font-size: 12px !important;
    padding: 6px 10px !important;
    border-radius: 6px !important;
    box-shadow: 0 4px 12px rgba(0,0,0,0.4) !important;
  }
  :global(.vis-navigation) {
    position: absolute;
    right: 10px;
    top: 10px;
  }
  :global(.vis-button) {
    background: #1a1a2e !important;
    border: 1px solid #3a3a5e !important;
    color: #e0e0e0 !important;
    border-radius: 4px !important;
    margin: 4px !important;
    box-shadow: none !important;
  }
  :global(.vis-button:hover) {
    background: #2a2a4e !important;
    border-color: #5a7aae !important;
  }
</style>
