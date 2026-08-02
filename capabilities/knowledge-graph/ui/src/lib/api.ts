// API client for the knowledge-graph capability.
// All calls go to the same origin (server.ts serves both UI and API).

const BASE = '';

export interface GraphNode {
  id: string;
  label: string;
  file_type: string;
  source_file: string;
  community: number;
  group: string;
}

export interface GraphEdge {
  from: string;
  to: string;
  label?: string;
}

export interface FullGraph {
  nodes: GraphNode[];
  links: GraphEdge[];
  directed: boolean;
  multigraph: boolean;
  built_at_commit?: string;
}

export interface GraphStats {
  nodes: number;
  edges: number;
  communities: number;
  built_at: string;
  corpus_files: number;
  doc_files: number;
}

export interface SearchResult {
  query: string;
  results: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface CommunityResult {
  community: number;
  members: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface NodeDetail {
  node: GraphNode;
  connections: Array<{
    relationship: string;
    node: GraphNode;
  }>;
}

async function fetchJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
  return res.json();
}

export function getGraph(): Promise<FullGraph> {
  return fetchJson<FullGraph>('/api/graph');
}

export function getStats(): Promise<GraphStats> {
  return fetchJson<GraphStats>('/api/graph/stats');
}

export function searchNodes(q: string): Promise<SearchResult> {
  return fetchJson<SearchResult>(`/api/graph/search?q=${encodeURIComponent(q)}`);
}

export function getCommunity(id: number): Promise<CommunityResult> {
  return fetchJson<CommunityResult>(`/api/graph/community/${id}`);
}

export function getNode(id: string): Promise<NodeDetail> {
  return fetchJson<NodeDetail>(`/api/graph/node/${encodeURIComponent(id)}`);
}
