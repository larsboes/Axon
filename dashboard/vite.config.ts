import { execFileSync, execSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig, type Plugin, type ProxyOptions } from "vite";
import {
  hasSameOrigin,
  installCommsProxyAuthorization,
  isMutation,
  loadCommsProxyCredential,
} from "./vite/comms-proxy-auth";

const AXON_ROOT = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const port = Number(process.env.AXON_PORT ?? 47117);
// What every visitor downloads before interacting with anything.
const APP_BUNDLE_LIMIT_BYTES = 500_000;
// A chunk nobody can reach without a dynamic import is not application weight, but it is
// still weight. Capped separately and higher: a diagram renderer is legitimately large, and
// what this guards against is one arriving unnoticed rather than one existing at all.
const LAZY_CHUNK_LIMIT_BYTES = 1_200_000;
// Renderers heavy enough that reaching the eager graph would be a regression nobody notices
// until every page is slow. One row per library rather than a second copy of the rule.
//
// `total` bounds the library across every chunk Rollup splits it into, measured with ~5%
// headroom so an upstream bump that doubles something has to be looked at. It is a
// footprint bound, not a per-load one, and the two differ by a lot for Mermaid: MapLibre
// arrives as one real chunk of 1.05 MB, while Mermaid self-splits by diagram type across 52
// chunks totalling 2.57 MB, of which a reader pulls the ~1.3 MB core plus only the diagram
// types actually on the page. What bounds any single download is LAZY_CHUNK_LIMIT_BYTES
// above; this bounds the library growing while nobody is watching.
const LAZY_VENDORS = [
  { label: "MapLibre", match: ["/maplibre-gl/"], total: 1_100_000 },
  { label: "Mermaid", match: ["/mermaid/", "/@mermaid-js/"], total: 2_700_000 },
];

interface RegistryEntry {
  name: string;
  kind: string;
  scope: "capability" | "spine";
  port: string;
  proxy_extra: string[];
  proxy_api_only: string;
}

// Lazy is a property of the import graph, not of a flag.
//
// This guard used to ask `chunk.isDynamicEntry`, and measure every chunk that was not
// MapLibre against the application limit. Both are wrong in the same direction, and adding
// Mermaid is what exposed it: `isDynamicEntry` is true only for a chunk that IS the target
// of a dynamic import, so a shared chunk Rollup splits OUT of one comes back false while
// still being unreachable without it. Mermaid produced exactly that -- three chunks totalling
// 2.2 MB, none of them reachable from an entry, all of them failing a limit named for the
// application. The lazy loading was correct; the measurement was not.
//
// So reachability is computed rather than asked for: walk `imports` from each entry chunk
// and never follow `dynamicImports`, because not following one is the entire point. What the
// walk reaches is what every visitor downloads, and that is what the application limit is
// about. Everything else is bounded too, just at a size a renderer can actually be.
function bundleGuard(): Plugin {
  return {
    name: "bundle-guard",
    generateBundle(_options, bundle) {
      const eager = new Set<string>();
      const walk = (fileName: string) => {
        if (eager.has(fileName)) return;
        const chunk = bundle[fileName];
        if (!chunk || chunk.type !== "chunk") return;
        eager.add(fileName);
        for (const dep of chunk.imports) walk(dep);
      };
      for (const output of Object.values(bundle)) {
        if (output.type === "chunk" && output.isEntry) walk(output.fileName);
      }

      const allChunks = Object.values(bundle).flatMap((output) =>
        output.type === "chunk" ? [output] : [],
      );
      const sizeOf = (chunk: (typeof allChunks)[number]) => Buffer.byteLength(chunk.code);

      for (const chunk of allChunks) {
        const isEager = eager.has(chunk.fileName);
        const limit = isEager ? APP_BUNDLE_LIMIT_BYTES : LAZY_CHUNK_LIMIT_BYTES;
        const bytes = sizeOf(chunk);
        if (bytes > limit) {
          this.error(
            `${chunk.fileName} is ${bytes} bytes; the limit for a ` +
              `${isEager ? "statically reachable" : "lazy"} chunk is ${limit}.`,
          );
        }
      }

      for (const vendor of LAZY_VENDORS) {
        const owned = allChunks.filter((chunk) =>
          Object.keys(chunk.modules).some((id) =>
            vendor.match.some((fragment) => id.includes(fragment)),
          ),
        );
        if (owned.length === 0) continue;

        const leaked = owned.find((chunk) => eager.has(chunk.fileName));
        if (leaked) {
          this.error(
            `${vendor.label} must remain lazy; ${leaked.fileName} is reachable from an entry ` +
              `without a dynamic import.`,
          );
        }

        const bytes = owned.reduce((total, chunk) => total + sizeOf(chunk), 0);
        if (bytes > vendor.total) {
          this.error(
            `${vendor.label} bundles total ${bytes} bytes; the limit is ${vendor.total}.`,
          );
        }
      }
    },
  };
}

// The proxy table is derived, not written. `tools/capability.sh registry` reads the
// service.toml manifests through tools/lib/toml.sh, the shell-side TOML parser
// (README.md#one-manifest-per-concern), so a capability's port is declared in exactly one file and this config,
// axon-status and the runner all read the same number.
//
// Read once at dev-server start: enabling a capability or moving a port means
// restarting the dashboard, which is honest — the shell's shape follows the machine's.
function registry(): RegistryEntry[] {
  const out = execFileSync(resolve(AXON_ROOT, "tools/capability.sh"), ["registry"], {
    encoding: "utf8",
  });
  return JSON.parse(out) as RegistryEntry[];
}

function buildProxy(): Record<string, ProxyOptions> {
  const proxy: Record<string, ProxyOptions> = {};
  const commsCredential = loadCommsProxyCredential(AXON_ROOT);

  if (!commsCredential.authorization) {
    console.warn(
      `[dashboard] Comms write proxy has no credential (${commsCredential.reason}); write routes remain fail-closed.`,
    );
  }

  for (const svc of registry()) {
    // The spine is this process; a capability with no port has no HTTP surface.
    if (svc.scope === "spine" || !svc.port) continue;
    const target = `http://127.0.0.1:${svc.port}`;

    // Uniform rule, no manifest field needed: /<name> reaches the capability with the
    // prefix stripped, so a capability's own contract never has to know it is proxied.
    const proxyPath =
      svc.proxy_api_only === "true" ? `/${svc.name}/api` : `/${svc.name}`;
    const options: ProxyOptions = {
      target,
      changeOrigin: true,
      rewrite: (path) => path.replace(new RegExp(`^/${svc.name}`), ""),
    };
    if (svc.name === "comms" && commsCredential.authorization) {
      const authorization = commsCredential.authorization;
      options.configure = (server) => installCommsProxyAuthorization(server, authorization);
    }
    proxy[proxyPath] = options;

    // Surfaces whose paths predate that rule (transit's /api, scouting's /discover)
    // pass through unstripped, declared per capability in its own manifest.
    for (const extra of svc.proxy_extra ?? []) {
      proxy[extra] = { target, changeOrigin: true };
    }
  }

  // The real LifeOS Life Dashboard (~/.claude/LIFEOS/PULSE/pulse.ts), an external
  // system this shell links to and never an Axon capability. Not in the registry, so
  // it stays declared here by hand.
  proxy["/lifeos-pulse"] = {
    target: "http://localhost:31337",
    changeOrigin: true,
    rewrite: (path) => path.replace(/^\/lifeos-pulse/, ""),
  };

  // macmon used to be hand-proxied here, on a hardcoded 9911, because it was not a
  // capability. It is capabilities/macmon now, so the registry loop above already gives it
  // /macmon from its own `port` — one fewer place for that number to be wrong.

  return proxy;
}

/** Top memory consumers on this machine, as JSON. Runs `ps` every request, so this
 *  is intentionally not a capability — it is a trivial dev-server convenience for the
 *  dashboard's Systems page hover detail. The response is ~500 bytes, uncached.
 *  
 *  Each entry: { pid, rss_mb, name }. `name` is the basename of the executable. */
function topProcesses(_req: import("http").IncomingMessage, res: import("http").ServerResponse) {
  try {
    const raw = execSync("ps -eo pid=,rss=,comm=", { encoding: "utf8", timeout: 3000 });
    const lines = raw.trim().split("\n");
    const procs = lines
      .map((l) => {
        const m = l.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
        if (!m) return null;
        const rssKb = parseInt(m[2], 10);
        // Filter out the ps process itself and tiny system processes
        if (rssKb < 100 * 1024) return null; // < 100 MB
        return { pid: parseInt(m[1], 10), rss_mb: Math.round(rssKb / 1024), name: m[3].split("/").pop() ?? m[3] };
      })
      .filter((p): p is NonNullable<typeof p> => p !== null)
      .sort((a, b) => b.rss_mb - a.rss_mb);
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(procs.slice(0, 15)));
  } catch {
    res.writeHead(500);
    res.end(JSON.stringify({ error: "failed to read processes" }));
  }
}

function guardCommsMutations(
  req: import("http").IncomingMessage,
  res: import("http").ServerResponse,
  next: () => void,
) {
  if (!req.url?.startsWith("/comms/") || !isMutation(req.method) || hasSameOrigin(req.headers)) {
    next();
    return;
  }
  res.writeHead(403, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ error: "cross-origin Comms mutations are not allowed" }));
}

// A function, not an object, so the proxy table is built only when a server is
// actually starting. `buildProxy()` shells out to tools/capability.sh and reads
// every manifest in the repo; a production build has no server, no need for the
// table, and — under Bazel — neither the script nor the manifests in its sandbox.
// Evaluating it at config load was what kept this app out of the build graph.
export default defineConfig(({ command }) => ({
  plugins: [sveltekit(), bundleGuard(), {
    name: "top-processes",
    configureServer(server) {
      server.middlewares.use(guardCommsMutations);
      server.middlewares.use("/api/top-processes", topProcesses);
    },
  }],
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true,
    ...(command === "serve" ? { proxy: buildProxy() } : {}),
  },
  preview: { host: "127.0.0.1", port, strictPort: true },
  build: {
    // Vite's own warning, silenced up to the largest size bundleGuard() will actually
    // allow — otherwise it fires on every lazy renderer chunk the guard has already
    // measured and accepted, and a warning that is always on is one nobody reads.
    // The guard, not this number, is what fails a build.
    chunkSizeWarningLimit: LAZY_CHUNK_LIMIT_BYTES / 1000,
  },
}));
