<script lang="ts">
  /**
   * The one map component in this shell. It owns the frame, the deferred / loading / failed
   * states and the observers; `./surface.svelte.ts` owns the MapLibre instance and outlives
   * this component so a route change does not rebuild one.
   *
   * A view brings its own `sources` and `layers` and nothing else. That is the modular half:
   * `/map`'s six spend/travel/people layers and `/travel`'s three trip layers are still
   * written where they are used, in the files that know what they mean.
   */
  import { onMount, untrack } from "svelte";
  import type { MapGeoJSONFeature } from "maplibre-gl";
  import { lease, type MapFeatureCollection, type MapLayerSpec, type MapLease } from "./surface";

  let {
    sources,
    layers,
    interactive = [],
    onFeatureClick,
    popupHtml,
    center,
    zoom,
    autoFit = true,
    /**
     * When set, the view refits every time this string changes; when absent it fits once, the
     * first time any feature arrives.
     *
     * Both behaviours already existed and both were right for their caller. `/travel` derives
     * its points from the filtered plan list AND from which row is highlighted, so hovering
     * rebuilt the array with the same places and a different `selected` flag — keyed on the id
     * list, a hover no longer flies the camera to the bounds it is already at, while changing
     * the plan filter still refits because that really is a different set of places.
     */
    fitKey,
    deferredLabel = "Map",
    /** Load immediately instead of waiting to be scrolled near. True where the map is the page. */
    eager = false,
  }: {
    sources: Record<string, MapFeatureCollection>;
    layers: MapLayerSpec[];
    interactive?: string[];
    onFeatureClick?: (layerId: string, feature: MapGeoJSONFeature) => void;
    popupHtml?: (layerId: string, properties: Record<string, unknown>) => string | null;
    center?: [number, number];
    zoom?: number;
    autoFit?: boolean;
    fitKey?: string;
    deferredLabel?: string;
    eager?: boolean;
  } = $props();

  let host: HTMLDivElement;
  let held: MapLease | undefined;
  let loadRequested = $state(false);
  let ready = $state(false);
  let failed = $state(false);
  let requestLoad = $state<() => void>(() => {});
  let fitted = "";

  /** Re-frame under whichever of the two rules this caller asked for. */
  function maybeFit(): void {
    if (!held || !autoFit) return;
    const key = fitKey ?? "once";
    if (key === fitted) return;
    if (held.fit(sources)) fitted = key;
  }

  export function flyTo(target: [number, number], targetZoom = 9): void {
    held?.flyTo(target, targetZoom);
  }

  onMount(() => {
    let disposed = false;
    let observer: IntersectionObserver | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let resizeFrame: number | undefined;

    // MapLibre reads the host dimensions when its canvas is created, and a map can mount
    // inside a conditional or detail layout whose box is legitimately 0x0 on that frame.
    // Controls are ordinary DOM and still appear in that state, while the WebGL canvas never
    // receives the resize that lets the first render finish.
    function resizeMap(): void {
      if (!held || disposed || resizeFrame !== undefined) return;
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = undefined;
        if (!disposed && host.clientWidth > 0 && host.clientHeight > 0) held?.resize();
      });
    }

    function loadMap(): void {
      if (loadRequested || disposed) return;
      loadRequested = true;
      observer?.disconnect();

      void lease(host, { ...(center ? { center } : {}), ...(zoom !== undefined ? { zoom } : {}) })
        .then((handle) => {
          if (disposed) {
            handle.release();
            return;
          }
          held = handle;
          handle.apply({
            sources: untrack(() => sources),
            layers: untrack(() => layers),
            interactive,
            ...(onFeatureClick ? { onFeatureClick } : {}),
            ...(popupHtml ? { popupHtml } : {}),
          });
          ready = true;
          resizeMap();
          maybeFit();
        })
        .catch(() => {
          failed = true;
        });
    }
    requestLoad = loadMap;

    if ("ResizeObserver" in window) {
      resizeObserver = new ResizeObserver(([entry]) => {
        if (entry && entry.contentRect.width > 0 && entry.contentRect.height > 0) resizeMap();
      });
      resizeObserver.observe(host);
    }

    if (eager || !("IntersectionObserver" in window)) {
      loadMap();
    } else {
      observer = new IntersectionObserver(([entry]) => entry?.isIntersecting && loadMap(), {
        rootMargin: "240px 0px",
      });
      observer.observe(host);
    }

    return () => {
      disposed = true;
      requestLoad = () => {};
      observer?.disconnect();
      resizeObserver?.disconnect();
      if (resizeFrame !== undefined) cancelAnimationFrame(resizeFrame);
      held?.release();
      held = undefined;
    };
  });

  $effect(() => {
    sources;
    if (!ready) return;
    held?.refresh(sources);
    maybeFit();
  });

  // Layers change far less often than data, and re-applying them tears down and rebuilds
  // every source, so this is deliberately keyed on the spec list rather than run beside the
  // data effect above.
  $effect(() => {
    layers;
    if (!ready || !held) return;
    held.apply({
      sources: untrack(() => sources),
      layers,
      interactive,
      ...(onFeatureClick ? { onFeatureClick } : {}),
      ...(popupHtml ? { popupHtml } : {}),
    });
  });
</script>

<div class="map-frame" aria-busy={loadRequested && !ready && !failed}>
  <div class="map" bind:this={host}></div>
  {#if !loadRequested}
    <div class="map-state map-deferred">
      <span>{deferredLabel}</span>
      <button type="button" onclick={requestLoad}>Load map</button>
    </div>
  {:else if !ready && !failed}
    <p class="map-state">Loading map…</p>
  {:else if failed}
    <div class="map-state map-deferred">
      <span>The map is currently unavailable.</span>
      <small>The panel keeps working without it.</small>
    </div>
  {/if}
</div>

<style>
  .map-frame {
    position: relative;
    height: 100%;
    min-height: 18rem;
    overflow: hidden;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
  }

  .map {
    position: absolute;
    inset: 0;
  }

  .map-state {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .map-deferred {
    align-content: center;
    gap: 0.75rem;
    background:
      radial-gradient(circle at 22% 36%, color-mix(in srgb, var(--accent) 18%, transparent) 0 3px, transparent 4px),
      radial-gradient(circle at 68% 58%, color-mix(in srgb, var(--accent) 18%, transparent) 0 3px, transparent 4px),
      linear-gradient(135deg, color-mix(in srgb, var(--card-bg) 92%, var(--accent)), var(--card-bg));
  }

  .map-deferred span {
    color: var(--text-secondary);
    font-weight: 650;
  }

  .map-deferred small {
    max-width: 24rem;
    text-align: center;
  }

  .map-deferred button {
    justify-self: center;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    padding: 0.55rem 0.9rem;
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }

  .map-deferred button:hover {
    border-color: var(--accent);
  }

  :global(.maplibregl-ctrl-attrib) {
    font-size: 0.625rem;
  }

  :global(.maplibregl-popup-content) {
    border-radius: var(--radius-sm);
    padding: 0.6rem 0.75rem;
    font-family: var(--font-sans);
    font-size: var(--text-xs);
    line-height: 1.45;
    color: #18181b;
  }
</style>
