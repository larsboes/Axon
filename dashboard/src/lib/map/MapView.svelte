<script lang="ts">
  /**
   * A generic MapLibre surface: named GeoJSON sources, declarative layers, click
   * handling. TripMap (`../travel/TripMap.svelte`) is the model — the lazy dynamic
   * import behind an IntersectionObserver, the resize dance, and the deferred /
   * loading / failed states are carried over unchanged, so maplibre-gl stays out
   * of the eager bundle (vite.config.ts, bundleGuard).
   *
   * `layers` is read once, when the style has loaded. `sources` is reactive: the
   * host swaps feature collections and this component setData's them in place.
   */
  import { onMount } from "svelte";
  import type {
    AddLayerObject,
    GeoJSONSource,
    LngLatLike,
    Map as MapLibreMap,
    MapGeoJSONFeature,
  } from "maplibre-gl";
  import { MAP_STYLE_URL } from "./style";

  /** Structural GeoJSON, so callers pass wire-typed collections without casts. */
  export interface MapFeature {
    type: "Feature";
    geometry: {
      type: "Point" | "LineString";
      coordinates: [number, number] | [number, number][];
    };
    properties: unknown;
  }

  export interface MapFeatureCollection {
    type: "FeatureCollection";
    features: MapFeature[];
  }

  export interface MapLayerSpec {
    id: string;
    type: "circle" | "line" | "symbol";
    source: string;
    filter?: unknown[];
    layout?: Record<string, unknown>;
    paint?: Record<string, unknown>;
  }

  let {
    sources,
    layers,
    interactive = [],
    onFeatureClick,
    popupHtml,
    center = [10.45, 51.16],
    zoom = 4.2,
    autoFit = true,
    deferredLabel = "Map",
  }: {
    sources: Record<string, MapFeatureCollection>;
    layers: MapLayerSpec[];
    /** Layer ids that take a pointer cursor and report clicks. */
    interactive?: string[];
    onFeatureClick?: (layerId: string, feature: MapGeoJSONFeature) => void;
    /** Pre-escaped HTML for a click popup; null/undefined opens none. */
    popupHtml?: (layerId: string, properties: Record<string, unknown>) => string | null;
    center?: [number, number];
    zoom?: number;
    /** Fit the view to the data once, the first time any feature arrives. */
    autoFit?: boolean;
    deferredLabel?: string;
  } = $props();

  let host: HTMLDivElement;
  let map: MapLibreMap | undefined;
  let mapLibrary: typeof import("maplibre-gl") | undefined;
  let loadRequested = $state(false);
  let ready = $state(false);
  let failed = $state(false);
  let requestLoad = $state<() => void>(() => {});
  let fitted = false;
  let popup: import("maplibre-gl").Popup | undefined;
  // One click can land on features in several interactive layers; the first
  // registered handler (the topmost layer, by `interactive` order) wins.
  let handledClick: MouseEvent | TouchEvent | undefined;

  export function flyTo(target: [number, number], targetZoom = 9): void {
    map?.flyTo({ center: target as LngLatLike, zoom: targetZoom, duration: 900 });
  }

  function eachPosition(visit: (lng: number, lat: number) => void): number {
    let count = 0;
    for (const collection of Object.values(sources)) {
      for (const feature of collection.features) {
        const { type, coordinates } = feature.geometry;
        const positions =
          type === "Point"
            ? [coordinates as [number, number]]
            : (coordinates as [number, number][]);
        for (const [lng, lat] of positions) {
          visit(lng, lat);
          count += 1;
        }
      }
    }
    return count;
  }

  function updateMap(): void {
    if (!map || !ready || !mapLibrary) return;
    for (const [id, data] of Object.entries(sources)) {
      (map.getSource(id) as GeoJSONSource | undefined)?.setData(
        data as Parameters<GeoJSONSource["setData"]>[0],
      );
    }

    if (!autoFit || fitted) return;
    const bounds = new mapLibrary.LngLatBounds();
    const positions = eachPosition((lng, lat) => bounds.extend([lng, lat]));
    if (positions === 0) return;
    fitted = true;
    map.fitBounds(bounds, { padding: 64, maxZoom: 10, duration: 500 });
  }

  onMount(() => {
    let disposed = false;
    let observer: IntersectionObserver | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let resizeFrame: number | undefined;

    // Same reason as TripMap: the host can legitimately measure 0x0 on the frame
    // the canvas is created, and only an explicit resize lets the render finish.
    function resizeMap(): void {
      if (!map || disposed || resizeFrame !== undefined) return;
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = undefined;
        if (!disposed && host.clientWidth > 0 && host.clientHeight > 0) {
          map?.resize();
        }
      });
    }

    function loadMap(): void {
      if (loadRequested || disposed) return;
      loadRequested = true;
      observer?.disconnect();

      void Promise.all([
        import("maplibre-gl"),
        import("maplibre-gl/dist/maplibre-gl.css"),
      ]).then(([maplibregl]) => {
        if (disposed) return;
        mapLibrary = maplibregl;
        map = new maplibregl.Map({
          container: host,
          style: MAP_STYLE_URL,
          center,
          zoom,
          cooperativeGestures: true,
          attributionControl: { compact: true },
        });
        resizeMap();
        map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
        map.on("load", () => {
          if (!map) return;
          for (const [id, data] of Object.entries(sources)) {
            map.addSource(id, {
              type: "geojson",
              data: data as Parameters<GeoJSONSource["setData"]>[0],
            });
          }
          for (const layer of layers) {
            map.addLayer(layer as AddLayerObject);
          }
          for (const layerId of interactive) {
            map.on("click", layerId, (event) => {
              const feature = event.features?.[0];
              if (!feature || event.originalEvent === handledClick) return;
              handledClick = event.originalEvent;
              onFeatureClick?.(layerId, feature);
              const html = popupHtml?.(layerId, feature.properties ?? {});
              if (html && mapLibrary && map) {
                popup?.remove();
                popup = new mapLibrary.Popup({ maxWidth: "300px", offset: 12 })
                  .setLngLat(event.lngLat)
                  .setHTML(html)
                  .addTo(map);
              }
            });
            map.on("mouseenter", layerId, () => {
              if (map) map.getCanvas().style.cursor = "pointer";
            });
            map.on("mouseleave", layerId, () => {
              if (map) map.getCanvas().style.cursor = "";
            });
          }
          ready = true;
          resizeMap();
          updateMap();
        });
      }).catch(() => {
        failed = true;
      });
    }
    requestLoad = loadMap;

    if ("ResizeObserver" in window) {
      resizeObserver = new ResizeObserver(([entry]) => {
        if (entry && entry.contentRect.width > 0 && entry.contentRect.height > 0) {
          resizeMap();
        }
      });
      resizeObserver.observe(host);
    }

    if ("IntersectionObserver" in window) {
      observer = new IntersectionObserver(
        ([entry]) => {
          if (entry?.isIntersecting) loadMap();
        },
        { rootMargin: "240px 0px" },
      );
      observer.observe(host);
    } else {
      loadMap();
    }

    return () => {
      disposed = true;
      requestLoad = () => {};
      observer?.disconnect();
      resizeObserver?.disconnect();
      if (resizeFrame !== undefined) cancelAnimationFrame(resizeFrame);
      map?.remove();
    };
  });

  $effect(() => {
    sources;
    updateMap();
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
    font-size: 0.75rem;
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
    font-size: 0.75rem;
    line-height: 1.45;
    color: #18181b;
  }
</style>
