<script lang="ts">
  import { onMount } from 'svelte';
  import GraphRenderer from '$lib/graph-renderer.svelte';
  import { getGraph, getStats, type GraphNode, type GraphEdge, type GraphStats } from '$lib/api';

  let graphData = $state<{ nodes: GraphNode[]; edges: GraphEdge[] } | null>(null);
  let stats = $state<GraphStats | null>(null);
  let error = $state<string>('');
  let loading = $state(true);
  let searchQuery = $state('');
  let selectedNode = $state<GraphNode | null>(null);
  let highlightTerm = $state('');

  onMount(async () => {
    try {
      const [graph, s] = await Promise.all([getGraph(), getStats()]);
      graphData = {
        nodes: graph.nodes ?? [],
        edges: graph.links ?? [],
      };
      stats = s;
    } catch (e: any) {
      error = e.message ?? 'Failed to load graph';
    } finally {
      loading = false;
    }
  });

  function handleSearch() {
    highlightTerm = searchQuery;
  }

  function handleNodeClick(nodeId: string) {
    if (!graphData) return;
    selectedNode = graphData.nodes.find(n => n.id === nodeId) ?? null;
  }

  function clearSelection() {
    selectedNode = null;
    highlightTerm = '';
    searchQuery = '';
  }

  function communityColor(community: number): string {
    const hue = ((community * 47) % 360);
    return `hsl(${hue}, 50%, 50%)`;
  }
</script>

{#if loading}
  <div class="loading">
    <div class="spinner"></div>
    <span>Loading codebase graph&hellip;</span>
  </div>
{:else if error}
  <div class="error">
    <h2>Graph not available</h2>
    <p>{error}</p>
    <p>Run <code>tools/graphify.sh</code> in the repo root first.</p>
  </div>
{:else if graphData}
  <div class="layout">
    <!-- Sidebar -->
    <aside class="sidebar">
      <!-- Stats -->
      {#if stats}
        <div class="stats">
          <div class="stat"><span class="stat-value">{stats.nodes}</span> nodes</div>
          <div class="stat"><span class="stat-value">{stats.edges}</span> edges</div>
          <div class="stat"><span class="stat-value">{stats.communities}</span> communities</div>
          <div class="stat"><span class="stat-value">{stats.corpus_files}</span> code files</div>
        </div>
        <div class="built-at">built at commit <code>{stats.built_at?.slice(0, 8) || '?'}</code></div>
      {/if}

      <!-- Search -->
      <div class="search-wrap">
        <input
          type="text"
          bind:value={searchQuery}
          onkeydown={(e) => e.key === 'Enter' && handleSearch()}
          placeholder="Search nodes&hellip;"
          class="search-input"
        />
        <button onclick={handleSearch} class="search-btn">Go</button>
      </div>

      <!-- Selected node detail -->
      {#if selectedNode}
        <div class="node-detail">
          <div class="node-detail-header">
            <strong>{selectedNode.label}</strong>
            <button onclick={clearSelection} class="close-btn">&times;</button>
          </div>
          <div class="node-detail-body">
            <div class="detail-row">
              <span class="detail-label">Type</span>
              <span>{selectedNode.file_type}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">File</span>
              <span class="detail-file" title={selectedNode.source_file}>
                {selectedNode.source_file.split('/').pop()}
              </span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Community</span>
              <span>
                <span class="community-dot" style="background: {communityColor(selectedNode.community)}"></span>
                {selectedNode.community}
              </span>
            </div>
          </div>
        </div>
      {:else}
        <div class="hint">
          <p>Click a node to inspect it.</p>
          <p>Double-click to zoom in.</p>
          <p>Drag to pan, scroll to zoom.</p>
        </div>
      {/if}
    </aside>

    <!-- Graph -->
    <div class="graph-area">
      <GraphRenderer
        nodes={graphData.nodes}
        edges={graphData.edges}
        highlight={highlightTerm}
        onNodeClick={handleNodeClick}
      />
    </div>
  </div>
{/if}

<style>
  .loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    gap: 1rem;
    color: #888;
  }
  .spinner {
    width: 32px;
    height: 32px;
    border: 3px solid #2a2a4e;
    border-top-color: #4E79A7;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    gap: 0.5rem;
    color: #f08080;
    text-align: center;
    padding: 2rem;
  }
  .error code {
    background: #1a1a2e;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 0.9em;
  }

  .layout {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .sidebar {
    width: 260px;
    background: #1a1a2e;
    border-right: 1px solid #2a2a4e;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    flex-shrink: 0;
  }

  .graph-area {
    flex: 1;
    min-width: 0;
    display: flex;
  }

  .stats {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    padding: 12px;
    border-bottom: 1px solid #2a2a4e;
  }
  .stat {
    font-size: 11px;
    color: #888;
  }
  .stat-value {
    display: block;
    font-size: 18px;
    font-weight: 700;
    color: #e0e0e0;
  }
  .built-at {
    padding: 6px 12px 10px;
    font-size: 10px;
    color: #666;
    border-bottom: 1px solid #2a2a4e;
  }
  .built-at code {
    font-size: 10px;
  }

  .search-wrap {
    display: flex;
    gap: 6px;
    padding: 10px 12px;
    border-bottom: 1px solid #2a2a4e;
  }
  .search-input {
    flex: 1;
    background: #0f0f1a;
    border: 1px solid #3a3a5e;
    color: #e0e0e0;
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 12px;
    outline: none;
  }
  .search-input:focus {
    border-color: #4E79A7;
  }
  .search-btn {
    background: #2d5f8a;
    border: none;
    color: #e0e0e0;
    padding: 6px 12px;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }
  .search-btn:hover {
    background: #3a7aae;
  }

  .node-detail {
    border-bottom: 1px solid #2a2a4e;
  }
  .node-detail-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 12px;
    background: #0f0f1a;
    border-bottom: 1px solid #2a2a4e;
    font-size: 13px;
  }
  .close-btn {
    background: none;
    border: none;
    color: #888;
    font-size: 18px;
    cursor: pointer;
    padding: 0 4px;
  }
  .close-btn:hover { color: #e0e0e0; }
  .node-detail-body {
    padding: 8px 12px;
    font-size: 12px;
  }
  .detail-row {
    display: flex;
    justify-content: space-between;
    padding: 4px 0;
    gap: 8px;
  }
  .detail-label {
    color: #888;
    white-space: nowrap;
  }
  .detail-file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 140px;
    text-align: right;
  }
  .community-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    vertical-align: middle;
    margin-right: 4px;
  }

  .hint {
    padding: 14px 12px;
    font-size: 12px;
    color: #666;
    line-height: 1.6;
  }
</style>
