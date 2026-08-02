<script lang="ts">
  import { onMount } from "svelte";
  import type { GeoJSONSource, Map as MapLibreMap } from "maplibre-gl";

  export interface MapPoint {
    id: string;
    groupId?: string;
    routeId?: string;
    label: string;
    latitude: number;
    longitude: number;
    kind: "origin" | "destination";
    phase?: "upcoming" | "past";
    selected: boolean;
  }

  let {
    points,
    onSelect,
  }: {
    points: MapPoint[];
    onSelect?: (groupId: string) => void;
  } = $props();
  let host: HTMLDivElement;
  let map: MapLibreMap | undefined;
  let mapLibrary: typeof import("maplibre-gl") | undefined;
  let loadRequested = $state(false);
  let ready = $state(false);
  let failed = $state(false);
  let requestLoad = $state<() => void>(() => {});

  const featureCollection = () => ({
    type: "FeatureCollection" as const,
    features: points.map((point) => ({
      type: "Feature" as const,
      properties: {
        id: point.id,
        groupId: point.groupId ?? point.id,
        label: point.label,
        kind: point.kind,
        phase: point.phase ?? "upcoming",
        selected: point.selected ? 1 : 0,
      },
      geometry: {
        type: "Point" as const,
        coordinates: [point.longitude, point.latitude],
      },
    })),
  });

  const routeCollection = () => {
    const routes = new Map<string, MapPoint[]>();
    for (const point of points) {
      const routeId = point.routeId ?? "active";
      routes.set(routeId, [...(routes.get(routeId) ?? []), point]);
    }
    return {
      type: "FeatureCollection" as const,
      features: [...routes.entries()]
        .filter(([, routePoints]) => routePoints.length > 1)
        .map(([routeId, routePoints]) => ({
          type: "Feature" as const,
          properties: {
            routeId,
            phase: routePoints[0].phase ?? "upcoming",
          },
          geometry: {
            type: "LineString" as const,
            coordinates: routePoints.map((point) => [point.longitude, point.latitude]),
          },
        })),
    };
  };

  function updateMap(): void {
    if (!map || !ready) return;
    (map.getSource("trip-points") as GeoJSONSource | undefined)?.setData(featureCollection());
    (map.getSource("trip-route") as GeoJSONSource | undefined)?.setData(routeCollection());

    if (points.length === 0) return;
    if (!mapLibrary) return;
    const bounds = points.reduce(
      (current, point) => current.extend([point.longitude, point.latitude]),
      new mapLibrary.LngLatBounds(),
    );
    map.fitBounds(bounds, { padding: 64, maxZoom: 10, duration: 500 });
  }

  onMount(() => {
    let disposed = false;
    let observer: IntersectionObserver | undefined;

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
          style: "https://tiles.openfreemap.org/styles/liberty",
          center: [10.45, 51.16],
          zoom: 4.2,
          cooperativeGestures: true,
          attributionControl: { compact: true },
        });
        map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
        map.on("load", () => {
          map?.addSource("trip-route", { type: "geojson", data: routeCollection() });
          map?.addLayer({
            id: "trip-route",
            type: "line",
            source: "trip-route",
            paint: {
              "line-color": [
                "match",
                ["get", "phase"],
                "past",
                "#71717a",
                "#0891b2",
              ],
              "line-width": 3,
              "line-opacity": 0.65,
              "line-dasharray": [1.5, 1.5],
            },
          });
          map?.addSource("trip-points", { type: "geojson", data: featureCollection() });
          map?.addLayer({
            id: "trip-points",
            type: "circle",
            source: "trip-points",
            paint: {
              "circle-radius": ["case", ["==", ["get", "selected"], 1], 9, 6],
              "circle-color": [
                "case",
                ["==", ["get", "selected"], 1],
                "#d97706",
                ["==", ["get", "phase"], "past"],
                "#71717a",
                ["==", ["get", "kind"], "origin"],
                "#18181b",
                "#06b6d4",
              ],
              "circle-stroke-color": "#ffffff",
              "circle-stroke-width": 2,
            },
          });
          map?.addLayer({
            id: "trip-labels",
            type: "symbol",
            source: "trip-points",
            layout: {
              "text-field": ["get", "label"],
              "text-size": 12,
              "text-offset": [0, 1.4],
              "text-anchor": "top",
            },
            paint: {
              "text-color": "#18181b",
              "text-halo-color": "#ffffff",
              "text-halo-width": 1.5,
            },
          });
          map?.on("click", "trip-points", (event) => {
            const groupId = event.features?.[0]?.properties?.groupId;
            if (typeof groupId === "string") onSelect?.(groupId);
          });
          map?.on("mouseenter", "trip-points", () => {
            if (map) map.getCanvas().style.cursor = "pointer";
          });
          map?.on("mouseleave", "trip-points", () => {
            if (map) map.getCanvas().style.cursor = "";
          });
          ready = true;
          updateMap();
        });
      }).catch(() => {
        failed = true;
      });
    }
    requestLoad = loadMap;

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
      map?.remove();
    };
  });

  $effect(() => {
    points;
    updateMap();
  });
</script>

<div class="map-frame" aria-busy={loadRequested && !ready && !failed}>
  <div class="map" bind:this={host}></div>
  {#if !loadRequested}
    <div class="map-state map-deferred">
      <span>{points.length} {points.length === 1 ? "place" : "places"} on the map</span>
      <button
        type="button"
        onclick={requestLoad}
      >
        Load map
      </button>
    </div>
  {:else if !ready && !failed}
    <p class="map-state">Loading map…</p>
  {:else if failed}
    <div class="map-state map-deferred">
      <span>The map is currently unavailable.</span>
      <small>Trips and legs remain fully available in the list.</small>
    </div>
  {:else if points.length === 0}
    <p class="map-state">These trips do not have map coordinates yet.</p>
  {/if}
</div>

<style>
  .map-frame {
    position: relative;
    min-height: 22rem;
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
    background-size: auto, auto, auto;
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
</style>
