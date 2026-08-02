// vis-network is loaded from a CDN <script> tag at runtime (graph-renderer.svelte), not
// imported, so it exists on `window` and nowhere in the module graph. This is where that
// fact is declared.
//
// It used to be declared inside the component's instance script instead, where `declare`
// is not valid Svelte — so svelte-check reported the misplaced modifier and every
// `window.vis` access as an unknown property, six errors that sat in main because no CI
// job type-checked this package (Axon#139).
//
// The shape is what the renderer actually calls, not vis-network's full surface: a
// hand-written subset that claims more than it uses would be a second, wrong copy of
// upstream's types.
declare global {
  interface Window {
    vis: {
      Network: new (
        container: HTMLElement,
        data: { nodes: unknown; edges: unknown },
        options: Record<string, unknown>,
      ) => VisNetwork;
      DataSet: new (data?: unknown[]) => unknown;
    };
  }

  interface VisNetwork {
    on(event: string, handler: (params: { nodes?: string[] }) => void): void;
    focus(nodeId: string, options?: { scale?: number; animation?: boolean }): void;
    selectNodes(nodeIds: string[]): void;
    destroy(): void;
  }
}

export {};
