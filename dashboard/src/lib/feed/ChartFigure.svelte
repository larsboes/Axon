<script lang="ts">
  /**
   * A figure drawn from the table the server pulled out of the item's source.
   *
   * The server sends **data, not a specification**: a title, an axis pair, a
   * mark and rows. This component compiles that into Vega-Lite. That boundary is
   * the point — a model that emitted Vega-Lite directly could reach into scales,
   * transforms and data URLs, and the palette would be whatever it felt like.
   * Here it can choose two things, both already validated: bar or line, and the
   * numbers, each of which had to appear verbatim in the source.
   *
   * Vega loads by dynamic import so its bundle is code-split onto the reader
   * route, the same shape as mermaid and maplibre-gl.
   *
   * **Single series, deliberately.** The figure palette is the low-chroma print
   * palette the operator's papers use; run it through a categorical-separation
   * check and even two of its hues fail the normal-vision floor. One measure
   * needs no categorical scale and no legend, so the question never arises. The
   * mark colour is the one hue that clears 3:1 against each surface: teal_dark
   * at 5.66:1 on paper, aqua_light at 9.59:1 on the dark card.
   *
   * The table below the figure is not decoration. Extracted numbers are a claim
   * about a source, and the table is where the reader checks it — which is also
   * the accessibility relief a chart owes when identity is not colour-alone.
   */
  import { onMount } from "svelte";
  import { ROOTS } from "$lib/feed/mermaid-theme";
  import type { ContentChartData } from "$lib/api";

  let { data }: { data: ContentChartData } = $props();

  let container = $state<HTMLDivElement | null>(null);
  let error = $state<string | null>(null);
  let dark = $state(false);
  let showTable = $state(false);
  let sequence = 0;

  const axisLabel = $derived(
    data.unit ? `${data.measure_label} (${data.unit})` : data.measure_label,
  );

  function spec(darkMode: boolean) {
    const mark = darkMode ? ROOTS.aquaLight : ROOTS.tealDark;
    const ink = darkMode ? ROOTS.paper : ROOTS.ink;
    const grid = darkMode ? "#2E3A3A" : ROOTS.grid;
    // Ordered categories are numbers stored as text; telling Vega they are
    // quantitative is what puts 2024 and 2026 the right distance apart instead
    // of evenly spaced like names.
    const categoryType = data.mark === "line" ? "quantitative" : "nominal";

    return {
      $schema: "https://vega.github.io/schema/vega-lite/v6.json",
      data: {
        values: data.rows.map((row) => ({
          category: data.mark === "line" ? Number(row.category) : row.category,
          value: row.value,
        })),
      },
      // Responsive width, fixed height: the reader column is the constraint.
      width: "container",
      height: 220,
      background: "transparent",
      layer: [
        data.mark === "line"
          ? { mark: { type: "line", color: mark, strokeWidth: 2, point: { filled: true, size: 64 } } }
          : {
              mark: {
                type: "bar",
                color: mark,
                cornerRadiusEnd: 4,
                // A two-category chart at container width draws two slabs
                // hundreds of pixels wide, which reads as a colour field rather
                // than a measurement. Few categories get an absolute cap; many
                // get a band fraction so they still fit.
                ...(data.rows.length <= 4 ? { width: 56 } : { width: { band: 0.7 } }),
              },
            },
        // Direct labels while there are few enough to place. Reading a value off
        // a gridline is work the figure can do for you, and at this count a
        // legend-free single series has nothing else to carry the numbers.
        ...(data.rows.length <= 8
          ? [
              {
                mark: {
                  type: "text" as const,
                  dy: data.mark === "line" ? -12 : -8,
                  color: ink,
                  fontSize: 11,
                },
                encoding: {
                  // `~g` drops trailing zeros, so 49.5 stays 49.5 and 60 stays 60.
                  text: { field: "value", type: "quantitative" as const, format: ".4~g" },
                },
              },
            ]
          : []),
      ],
      encoding: {
        x: {
          field: "category",
          type: categoryType,
          title: data.category_label || null,
          axis: { labelAngle: 0, labelLimit: 120 },
        },
        y: {
          field: "value",
          type: "quantitative" as const,
          title: axisLabel || null,
        },
        tooltip: [
          { field: "category", type: categoryType, title: data.category_label || "Category" },
          { field: "value", type: "quantitative" as const, title: axisLabel || "Value" },
        ],
      },
      config: {
        // Recessive axes and grid; the marks are the only loud thing.
        axis: {
          labelColor: ink,
          titleColor: ink,
          gridColor: grid,
          domainColor: grid,
          tickColor: grid,
          labelFontSize: 12,
          titleFontSize: 12,
          titleFontWeight: 500 as const,
        },
        view: { stroke: null },
        font: "system-ui, -apple-system, sans-serif",
      },
    };
  }

  async function draw(darkMode: boolean) {
    if (!container) return;
    const run = ++sequence;
    error = null;
    try {
      const embed = (await import("vega-embed")).default;
      if (run !== sequence || !container) return;
      await embed(container, spec(darkMode) as never, {
        actions: false,
        renderer: "svg",
      });
    } catch (cause) {
      if (run !== sequence) return;
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
    if (container && data.rows.length) void draw(dark);
  });
</script>

<figure class="chart" class:dark>
  {#if data.title}<figcaption class="chart-title">{data.title}</figcaption>{/if}
  <div class="chart-plot" bind:this={container}></div>
  {#if error}<p class="chart-error">Could not draw this: {error}</p>{/if}

  {#if data.note}<p class="chart-note">{data.note}</p>{/if}

  <details class="chart-table" bind:open={showTable}>
    <summary>{showTable ? "Hide" : "Show"} the {data.rows.length} extracted values</summary>
    <table>
      <thead>
        <tr>
          <th scope="col">{data.category_label || "Category"}</th>
          <th scope="col">{axisLabel || "Value"}</th>
        </tr>
      </thead>
      <tbody>
        {#each data.rows as row (row.category)}
          <tr>
            <th scope="row">{row.category}</th>
            <td>{row.value}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="chart-provenance">
      Every value here was found verbatim in the source text before it was drawn. Anything the
      model reported that the source does not contain was dropped.
    </p>
  </details>
</figure>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin: 0;
    padding: 1.25rem 1rem 1rem;
    border: 1px solid #ddd6cf; /* palette root: grid */
    border-radius: var(--radius-md);
    background: #fbfaf7; /* palette root: paper */
    color: #1f2727; /* palette root: ink */
  }

  .chart.dark {
    border-color: var(--card-border);
    background: var(--card-bg);
    color: var(--text-primary);
  }

  .chart-title {
    margin: 0;
    font-size: 0.9rem;
    font-weight: 600;
  }

  .chart-plot {
    width: 100%;
    overflow-x: auto;
  }

  .chart-note,
  .chart-provenance {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.55;
    opacity: 0.72;
  }

  .chart-error {
    margin: 0;
    color: var(--danger);
    font-size: 0.8rem;
  }

  .chart-table summary {
    cursor: pointer;
    font-size: 0.75rem;
    opacity: 0.72;
  }

  .chart-table table {
    width: 100%;
    margin: 0.65rem 0;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  .chart-table th,
  .chart-table td {
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid #ddd6cf;
    text-align: left;
    font-weight: 400;
  }

  .chart.dark .chart-table th,
  .chart.dark .chart-table td {
    border-bottom-color: var(--card-border);
  }

  .chart-table thead th {
    font-weight: 600;
    opacity: 0.72;
  }

  .chart-table td {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
</style>
